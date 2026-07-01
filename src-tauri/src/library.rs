//! Watched-folder scanning: walk the clips folder (recursing into per-game
//! subfolders) and surface media files newest-first for the library grid.
//! Everything Klipt can produce shows back up here — videos, the GIF/WebP
//! exports, and audio-only exports — each tagged with a media kind so the
//! frontend can render and open it appropriately.

use std::cmp::Reverse;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Serialize;

pub(crate) const VIDEO_EXTS: [&str; 6] = ["mp4", "mov", "mkv", "avi", "webm", "m4v"];
/// Animated-image outputs (Klipt's own GIF/WebP exports). Rendered natively by
/// the webview, so the grid shows them without any ffmpeg work.
pub(crate) const ANIM_EXTS: [&str; 2] = ["gif", "webp"];
/// Audio-only files (Klipt's M4A/MP3 exports first, plus common lossless/open
/// formats so a stray recording still shows up).
pub(crate) const AUDIO_EXTS: [&str; 6] = ["m4a", "mp3", "wav", "ogg", "flac", "opus"];

/// Classify a file extension into the media kind the frontend keys its card
/// rendering and open behaviour on. `None` = not a media file Klipt lists.
fn media_kind(ext: &str) -> Option<&'static str> {
    if VIDEO_EXTS.iter().any(|v| ext.eq_ignore_ascii_case(v)) {
        return Some("video");
    }
    if ANIM_EXTS.iter().any(|v| ext.eq_ignore_ascii_case(v)) {
        return Some("anim");
    }
    if AUDIO_EXTS.iter().any(|v| ext.eq_ignore_ascii_case(v)) {
        return Some("audio");
    }
    None
}

/// Safety ceiling on how many Clips `list_recent_clips` returns. The frontend
/// virtualizes the grid, so this isn't about render cost — it bounds the scan's
/// payload/memory if a watched folder is pointed somewhere pathological. Far
/// above any realistic game-clip library.
const MAX_CLIPS: usize = 50_000;

/// A Clip surfaced in the recent-clips list.
#[derive(Serialize)]
pub(crate) struct ClipEntry {
    path: String,
    name: String,
    /// Parent folder name — ShadowPlay stores per-game, so this is the game.
    game: String,
    modified: u64,
    size_bytes: u64,
    /// Media kind: "video" | "anim" (GIF/WebP) | "audio". Drives card rendering
    /// and what opening the entry does (editor vs viewer).
    kind: &'static str,
}

/// List recent media in the watched folder (recursing into per-game subfolders),
/// newest first. When a separate output folder is configured it is scanned too
/// (deduplicated), so exports always show up in the overview even when they are
/// written outside the watched tree. Declared `async` so Tauri runs the
/// directory walk on its async runtime rather than the main thread, keeping the
/// UI responsive during the scan.
#[tauri::command]
pub(crate) async fn list_recent_clips(
    folder: String,
    output_dir: Option<String>,
) -> Result<Vec<ClipEntry>, String> {
    // The walk is blocking (synchronous `read_dir` + `metadata`), so run it on a
    // blocking thread rather than tying up an async-runtime worker for the whole
    // scan — keeps the runtime free to service thumbnail/probe commands meanwhile.
    tauri::async_runtime::spawn_blocking(move || scan_library(&folder, output_dir.as_deref()))
        .await
        .map_err(|e| e.to_string())
}

/// The full library scan: the watched folder plus (when set) the output folder
/// as a second root, deduplicated by path and sorted newest-first. Synchronous
/// and filesystem-only, so it's unit-testable without a Tauri runtime.
fn scan_library(folder: &str, output_dir: Option<&str>) -> Vec<ClipEntry> {
    let mut entries = Vec::new();
    collect_clips(&PathBuf::from(folder), 0, &mut entries);
    // The output folder may live outside the watched tree; scan it as a second
    // root so exports are never invisible. When it's nested inside the watched
    // folder the walk found everything already — the path-keyed dedup below
    // drops the duplicates either way.
    if let Some(out) = output_dir.map(str::trim).filter(|d| !d.is_empty()) {
        let mut extra = Vec::new();
        collect_clips(&PathBuf::from(out), 0, &mut extra);
        // Files sitting directly in the output root have the output folder
        // itself as their parent — label those "Exports" so they group sensibly
        // in the game filter instead of leaking the folder name.
        let out_root = PathBuf::from(out);
        for e in &mut extra {
            if Path::new(&e.path).parent() == Some(out_root.as_path()) {
                e.game = "Exports".into();
            }
        }
        let seen: HashSet<String> = entries.iter().map(|e| e.path.clone()).collect();
        entries.extend(extra.into_iter().filter(|e| !seen.contains(&e.path)));
    }
    entries.sort_by_key(|e| Reverse(e.modified));
    // The whole library is returned (the frontend windows the render, so card
    // count no longer bounds DOM cost) — capped only as a safety valve against
    // a watched folder pointed at something pathological. The scan stays
    // sub-second into the thousands and runs off the UI thread.
    entries.truncate(MAX_CLIPS);
    entries
}

fn collect_clips(dir: &PathBuf, depth: usize, out: &mut Vec<ClipEntry>) {
    if depth > 3 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        // Skip symlinks / junctions (reparse points): following one risks a loop
        // back to an ancestor or wandering off into an unrelated tree. We only
        // want the real on-disk subfolders of the watched library.
        if entry.file_type().map(|t| t.is_symlink()).unwrap_or(false) {
            continue;
        }
        if path.is_dir() {
            collect_clips(&path, depth + 1, out);
            continue;
        }
        // eq_ignore_ascii_case (inside media_kind) avoids allocating a
        // lowercased String per file.
        let Some(kind) = path
            .extension()
            .and_then(|s| s.to_str())
            .and_then(media_kind)
        else {
            continue;
        };
        let Ok(meta) = entry.metadata() else { continue };
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let game = path
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        out.push(ClipEntry {
            path: path.to_string_lossy().to_string(),
            name: path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
            game,
            modified,
            size_bytes: meta.len(),
            kind,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    // A fresh, collision-free temp dir per call: tests in this crate run
    // concurrently and have re-run, so a fixed name races against itself.
    fn unique_temp_dir(label: &str) -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("klipt_test_{label}_{}_{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn collect_clips_finds_videos_recursively_and_skips_non_videos() {
        let root = unique_temp_dir("collect_clips");
        let game = root.join("Apex Legends");
        std::fs::create_dir_all(&game).unwrap();
        // two videos in a per-game subfolder + one non-video, one video at root
        std::fs::write(game.join("clip1.mp4"), b"x").unwrap();
        std::fs::write(game.join("clip2.MKV"), b"x").unwrap(); // case-insensitive ext
        std::fs::write(game.join("notes.txt"), b"x").unwrap();
        std::fs::write(root.join("loose.webm"), b"x").unwrap();

        let mut out = Vec::new();
        collect_clips(&root, 0, &mut out);

        assert_eq!(out.len(), 3, "should find 3 videos, skip the .txt");
        let names: std::collections::HashSet<_> = out.iter().map(|e| e.name.clone()).collect();
        assert!(names.contains("clip1.mp4"));
        assert!(names.contains("clip2.MKV"));
        assert!(names.contains("loose.webm"));
        assert!(!names.contains("notes.txt"));
        // the per-game subfolder name is recorded as the game
        let apex = out.iter().find(|e| e.name == "clip1.mp4").unwrap();
        assert_eq!(apex.game, "Apex Legends");
        assert_eq!(apex.kind, "video");

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn media_kind_classifies_all_supported_extensions() {
        assert_eq!(media_kind("mp4"), Some("video"));
        assert_eq!(media_kind("MKV"), Some("video")); // case-insensitive
        assert_eq!(media_kind("gif"), Some("anim"));
        assert_eq!(media_kind("WebP"), Some("anim"));
        assert_eq!(media_kind("m4a"), Some("audio"));
        assert_eq!(media_kind("MP3"), Some("audio"));
        assert_eq!(media_kind("txt"), None);
        assert_eq!(media_kind(""), None);
    }

    #[test]
    fn collect_clips_surfaces_gif_and_audio_exports_with_their_kind() {
        let root = unique_temp_dir("collect_kinds");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("moment.gif"), b"x").unwrap();
        std::fs::write(root.join("loop.webp"), b"x").unwrap();
        std::fs::write(root.join("callout.m4a"), b"x").unwrap();
        std::fs::write(root.join("song.mp3"), b"x").unwrap();

        let mut out = Vec::new();
        collect_clips(&root, 0, &mut out);

        let kind_of = |n: &str| out.iter().find(|e| e.name == n).map(|e| e.kind);
        assert_eq!(kind_of("moment.gif"), Some("anim"));
        assert_eq!(kind_of("loop.webp"), Some("anim"));
        assert_eq!(kind_of("callout.m4a"), Some("audio"));
        assert_eq!(kind_of("song.mp3"), Some("audio"));

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn scan_library_merges_a_separate_output_dir_and_labels_root_exports() {
        let watched = unique_temp_dir("scan_watched");
        let out = unique_temp_dir("scan_out");
        let game = watched.join("Apex Legends");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::create_dir_all(out.join("older")).unwrap();
        std::fs::write(game.join("clip.mp4"), b"x").unwrap();
        std::fs::write(out.join("clip_trim.mp4"), b"x").unwrap();
        std::fs::write(out.join("clip_gif.gif"), b"x").unwrap();
        // A file in a subfolder of the output dir keeps its folder name as game.
        std::fs::write(out.join("older").join("old_trim.mp4"), b"x").unwrap();

        let entries = scan_library(&watched.to_string_lossy(), Some(&out.to_string_lossy()));

        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"clip.mp4"));
        assert!(names.contains(&"clip_trim.mp4"));
        assert!(names.contains(&"clip_gif.gif"));
        assert!(names.contains(&"old_trim.mp4"));
        // Root-level exports group under "Exports"; nested keep their folder.
        let game_of = |n: &str| {
            entries
                .iter()
                .find(|e| e.name == n)
                .map(|e| e.game.clone())
                .unwrap()
        };
        assert_eq!(game_of("clip_trim.mp4"), "Exports");
        assert_eq!(game_of("clip_gif.gif"), "Exports");
        assert_eq!(game_of("old_trim.mp4"), "older");
        assert_eq!(game_of("clip.mp4"), "Apex Legends");

        std::fs::remove_dir_all(&watched).unwrap();
        std::fs::remove_dir_all(&out).unwrap();
    }

    #[test]
    fn scan_library_dedupes_an_output_dir_nested_inside_the_watched_folder() {
        let watched = unique_temp_dir("scan_nested");
        let out = watched.join("exports");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join("clip_trim.mp4"), b"x").unwrap();

        let entries = scan_library(&watched.to_string_lossy(), Some(&out.to_string_lossy()));

        // Found by both roots, listed once (the watched-tree walk wins, so the
        // game label stays the folder name rather than "Exports").
        assert_eq!(entries.len(), 1, "nested output dir must not duplicate");
        assert_eq!(entries[0].name, "clip_trim.mp4");

        std::fs::remove_dir_all(&watched).unwrap();
    }

    #[test]
    fn scan_library_without_output_dir_matches_plain_walk() {
        let watched = unique_temp_dir("scan_plain");
        std::fs::create_dir_all(&watched).unwrap();
        std::fs::write(watched.join("a.mp4"), b"x").unwrap();

        // None and blank both mean "no second root".
        assert_eq!(scan_library(&watched.to_string_lossy(), None).len(), 1);
        assert_eq!(
            scan_library(&watched.to_string_lossy(), Some("  ")).len(),
            1
        );

        std::fs::remove_dir_all(&watched).unwrap();
    }

    // Symlink creation is portable enough on Unix (CI) to assert the reparse-point
    // skip directly; on Windows it needs privilege, so we rely on the same
    // `is_symlink()` guard there without a dedicated test.
    #[cfg(unix)]
    #[test]
    fn collect_clips_does_not_follow_a_symlink_loop() {
        let root = unique_temp_dir("collect_symlink_loop");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("real.mp4"), b"x").unwrap();
        // A child that links back to its own parent — a classic walk loop.
        std::os::unix::fs::symlink(&root, root.join("loop")).unwrap();

        let mut out = Vec::new();
        collect_clips(&root, 0, &mut out);

        // The loop is skipped, so the single real video is found exactly once
        // (and the walk terminates instead of re-entering via the link).
        assert_eq!(
            out.len(),
            1,
            "symlink loop must not be followed or double-counted"
        );
        assert_eq!(out[0].name, "real.mp4");

        std::fs::remove_dir_all(&root).unwrap();
    }
}
