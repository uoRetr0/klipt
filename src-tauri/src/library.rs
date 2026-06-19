//! Watched-folder scanning: walk the clips folder (recursing into per-game
//! subfolders) and surface video files newest-first for the library grid.

use std::cmp::Reverse;
use std::path::PathBuf;

use serde::Serialize;

pub(crate) const VIDEO_EXTS: [&str; 6] = ["mp4", "mov", "mkv", "avi", "webm", "m4v"];

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
}

/// List recent Clips in the watched folder (recursing into per-game subfolders),
/// newest first. Declared `async` so Tauri runs the directory walk on its async
/// runtime rather than the main thread, keeping the UI responsive during the scan.
#[tauri::command]
pub(crate) async fn list_recent_clips(folder: String) -> Result<Vec<ClipEntry>, String> {
    // The walk is blocking (synchronous `read_dir` + `metadata`), so run it on a
    // blocking thread rather than tying up an async-runtime worker for the whole
    // scan — keeps the runtime free to service thumbnail/probe commands meanwhile.
    tauri::async_runtime::spawn_blocking(move || {
        let mut entries = Vec::new();
        collect_clips(&PathBuf::from(&folder), 0, &mut entries);
        entries.sort_by_key(|e| Reverse(e.modified));
        // The whole library is returned (the frontend windows the render, so card
        // count no longer bounds DOM cost) — capped only as a safety valve against
        // a watched folder pointed at something pathological. The scan stays
        // sub-second into the thousands and runs off the UI thread.
        entries.truncate(MAX_CLIPS);
        entries
    })
    .await
    .map_err(|e| e.to_string())
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
        if path.is_dir() {
            collect_clips(&path, depth + 1, out);
            continue;
        }
        let is_video = path
            .extension()
            .and_then(|s| s.to_str())
            // eq_ignore_ascii_case avoids allocating a lowercased String per file.
            .map(|e| VIDEO_EXTS.iter().any(|v| e.eq_ignore_ascii_case(v)))
            .unwrap_or(false);
        if !is_video {
            continue;
        }
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
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_clips_finds_videos_recursively_and_skips_non_videos() {
        let root = std::env::temp_dir().join("klipt_test_collect_clips");
        let _ = std::fs::remove_dir_all(&root);
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

        std::fs::remove_dir_all(&root).unwrap();
    }
}
