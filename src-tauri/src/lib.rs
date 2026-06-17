use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tauri_plugin_shell::process::Output;
use tauri_plugin_shell::ShellExt;

const VIDEO_EXTS: [&str; 6] = ["mp4", "mov", "mkv", "avi", "webm", "m4v"];

/// Metadata about a source Clip, used to drive the timeline.
#[derive(Serialize)]
struct ClipInfo {
    duration: f64,
    width: u32,
    height: u32,
    size_bytes: u64,
}

/// A Clip surfaced in the recent-clips list.
#[derive(Serialize)]
struct ClipEntry {
    path: String,
    name: String,
    /// Parent folder name — ShadowPlay stores per-game, so this is the game.
    game: String,
    modified: u64,
    size_bytes: u64,
}

/// Result of a Trim or Compress.
#[derive(Serialize)]
struct TrimResult {
    path: String,
    size_bytes: u64,
    encoder: Option<String>,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct Settings {
    watched_folder: Option<String>,
}

// --- ffmpeg helpers -------------------------------------------------------

async fn run_ffmpeg(app: &AppHandle, args: Vec<String>) -> Result<Output, String> {
    app.shell()
        .sidecar("ffmpeg")
        .map_err(|e| e.to_string())?
        .args(args)
        .output()
        .await
        .map_err(|e| e.to_string())
}

/// Sanitize a user-supplied output name into a bare file stem (no path, no ext).
fn clean_stem(requested: Option<&str>, default_stem: &str) -> String {
    let raw = requested.map(|s| s.trim()).unwrap_or("");
    if raw.is_empty() {
        return default_stem.to_string();
    }
    // Drop any directory components and strip a trailing extension the user typed.
    let base = Path::new(raw)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(raw);
    let base = Path::new(base)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(base);
    let cleaned: String = base
        .chars()
        .filter(|c| !matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'))
        .collect();
    let cleaned = cleaned.trim().to_string();
    if cleaned.is_empty() {
        default_stem.to_string()
    } else {
        cleaned
    }
}

/// Resolve a collision-free output path next to the source.
fn resolve_output(parent: &Path, stem: &str, ext: &str) -> PathBuf {
    let mut out = parent.join(format!("{stem}.{ext}"));
    let mut n = 2;
    while out.exists() {
        out = parent.join(format!("{stem}_{n}.{ext}"));
        n += 1;
    }
    out
}

/// Compute the video bitrate ladder (kbps) for a size-targeted encode.
/// Returns (video_kbps, maxrate_kbps, bufsize_kbps). `dur` is the Region length
/// in seconds and must be > 0 (callers guarantee this via the end > start guard).
fn size_target_bitrate(target_mb: f64, dur: f64, audio_kbps: f64) -> (f64, f64, f64) {
    let target = target_mb.max(1.0);
    // Aim slightly under target to avoid overshoot. kbps = KB*8/sec.
    let total_kbps = (target * 1024.0 * 8.0 / dur) * 0.95;
    let v_kbps = (total_kbps - audio_kbps).max(300.0);
    (v_kbps, v_kbps * 1.45, v_kbps * 2.0)
}

fn file_size(p: &str) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

/// Probe just the duration of a Clip (seconds), best-effort (0.0 on failure).
async fn probe_duration(app: &AppHandle, path: &str) -> f64 {
    let Ok(child) = app.shell().sidecar("ffprobe") else {
        return 0.0;
    };
    let out = child
        .args([
            "-v", "error", "-show_entries", "format=duration", "-of",
            "default=nw=1:nk=1", path,
        ])
        .output()
        .await;
    out.ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<f64>().ok())
        .filter(|d| d.is_finite() && *d > 0.0)
        .unwrap_or(0.0)
}

// --- commands -------------------------------------------------------------

/// Probe a Clip for the duration and dimensions the UI needs.
#[tauri::command]
async fn probe_clip(app: AppHandle, path: String) -> Result<ClipInfo, String> {
    let out = app
        .shell()
        .sidecar("ffprobe")
        .map_err(|e| e.to_string())?
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "format=duration:stream=width,height",
            "-of",
            "json",
            &path,
        ])
        .output()
        .await
        .map_err(|e| e.to_string())?;

    if !out.status.success() {
        return Err(format!(
            "ffprobe failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }

    let v: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
    let duration = v["format"]["duration"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
    let stream = &v["streams"][0];
    let width = stream["width"].as_u64().unwrap_or(0) as u32;
    let height = stream["height"].as_u64().unwrap_or(0) as u32;

    Ok(ClipInfo {
        duration,
        width,
        height,
        size_bytes: file_size(&path),
    })
}

/// Losslessly trim the Region [start, end] out of a Clip via stream-copy.
/// Keeps every video and audio stream; never overwrites an existing file.
#[tauri::command]
async fn trim_clip(
    app: AppHandle,
    path: String,
    start: f64,
    end: f64,
    output_name: Option<String>,
) -> Result<TrimResult, String> {
    if !(end > start) {
        return Err("End point must be after the start point.".into());
    }

    let input = PathBuf::from(&path);
    let parent = input.parent().ok_or("Could not resolve the clip's folder.")?;
    let src_stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or("Could not read the clip's name.")?;
    let ext = input.extension().and_then(|s| s.to_str()).unwrap_or("mp4");

    let stem = clean_stem(output_name.as_deref(), &format!("{src_stem}_trim"));
    let out_path = resolve_output(parent, &stem, ext);
    let out_str = out_path.to_string_lossy().to_string();
    let dur = end - start;

    // -ss before -i: fast input seek to the nearest keyframe <= start.
    // -c copy: no re-encode. -map 0:v? / 0:a?: keep all video + audio streams.
    let args = vec![
        "-ss".into(),
        format!("{start}"),
        "-i".into(),
        path.clone(),
        "-t".into(),
        format!("{dur}"),
        "-map".into(),
        "0:v?".into(),
        "-map".into(),
        "0:a?".into(),
        "-c".into(),
        "copy".into(),
        "-avoid_negative_ts".into(),
        "make_zero".into(),
        "-y".into(),
        out_str.clone(),
    ];

    let output = run_ffmpeg(&app, args).await?;
    if !output.status.success() {
        return Err(format!(
            "ffmpeg failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    Ok(TrimResult {
        size_bytes: file_size(&out_str),
        path: out_str,
        encoder: None,
    })
}

/// Re-encode the Region to a smaller, shareable file.
/// mode = "size" (hit `target_mb`) or "quality" (use a CRF/CQ level).
/// Encoder is auto: try NVENC (GPU), fall back to libx264 (CPU).
#[tauri::command]
async fn compress_clip(
    app: AppHandle,
    path: String,
    start: f64,
    end: f64,
    output_name: Option<String>,
    mode: String,
    target_mb: Option<f64>,
    quality: Option<String>,
) -> Result<TrimResult, String> {
    if !(end > start) {
        return Err("End point must be after the start point.".into());
    }
    let dur = end - start;

    let input = PathBuf::from(&path);
    let parent = input.parent().ok_or("Could not resolve the clip's folder.")?;
    let src_stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or("Could not read the clip's name.")?;

    let stem = clean_stem(output_name.as_deref(), &format!("{src_stem}_small"));
    // Always output mp4 for maximum share compatibility (Discord, browsers).
    let out_path = resolve_output(parent, &stem, "mp4");
    let out_str = out_path.to_string_lossy().to_string();

    const AUDIO_KBPS: f64 = 128.0;

    // Build the encoder-specific video args for a given encoder.
    let video_args = |encoder: &str| -> Vec<String> {
        let mut a: Vec<String> = Vec::new();
        a.push("-c:v".into());
        a.push(encoder.into());
        if mode == "size" {
            let target = target_mb.unwrap_or(25.0);
            let (v_kbps, maxrate, bufsize) = size_target_bitrate(target, dur, AUDIO_KBPS);
            if encoder == "h264_nvenc" {
                a.extend(["-preset".into(), "p5".into(), "-rc".into(), "vbr".into()]);
            } else {
                a.extend(["-preset".into(), "medium".into()]);
            }
            a.extend([
                "-b:v".into(),
                format!("{v_kbps:.0}k"),
                "-maxrate".into(),
                format!("{maxrate:.0}k"),
                "-bufsize".into(),
                format!("{bufsize:.0}k"),
            ]);
        } else {
            // quality mode -> CQ (nvenc) / CRF (x264). lower = better/larger.
            let q = quality.as_deref().unwrap_or("medium");
            let level = match q {
                "high" => 20,
                "low" => 30,
                _ => 24,
            };
            if encoder == "h264_nvenc" {
                a.extend([
                    "-preset".into(),
                    "p5".into(),
                    "-rc".into(),
                    "vbr".into(),
                    "-cq".into(),
                    level.to_string(),
                    "-b:v".into(),
                    "0".into(),
                ]);
            } else {
                a.extend([
                    "-preset".into(),
                    "medium".into(),
                    "-crf".into(),
                    level.to_string(),
                ]);
            }
        }
        a
    };

    let build = |encoder: &str| -> Vec<String> {
        let mut args: Vec<String> = vec![
            "-ss".into(),
            format!("{start}"),
            "-i".into(),
            path.clone(),
            "-t".into(),
            format!("{dur}"),
            "-map".into(),
            "0:v:0".into(),
            "-map".into(),
            "0:a?".into(),
        ];
        args.extend(video_args(encoder));
        args.extend([
            "-pix_fmt".into(),
            "yuv420p".into(),
            "-c:a".into(),
            "aac".into(),
            "-b:a".into(),
            format!("{AUDIO_KBPS:.0}k"),
            "-movflags".into(),
            "+faststart".into(),
            "-y".into(),
            out_str.clone(),
        ]);
        args
    };

    // Try GPU first, fall back to CPU.
    let nvenc = run_ffmpeg(&app, build("h264_nvenc")).await?;
    if nvenc.status.success() {
        return Ok(TrimResult {
            size_bytes: file_size(&out_str),
            path: out_str,
            encoder: Some("NVENC (GPU)".into()),
        });
    }

    let x264 = run_ffmpeg(&app, build("libx264")).await?;
    if x264.status.success() {
        return Ok(TrimResult {
            size_bytes: file_size(&out_str),
            path: out_str,
            encoder: Some("x264 (CPU)".into()),
        });
    }

    Err(format!(
        "Compression failed: {}",
        String::from_utf8_lossy(&x264.stderr)
    ))
}

/// List recent Clips in the watched folder (recursing into per-game subfolders),
/// newest first.
#[tauri::command]
fn list_recent_clips(folder: String) -> Result<Vec<ClipEntry>, String> {
    let mut entries = Vec::new();
    collect_clips(&PathBuf::from(&folder), 0, &mut entries);
    entries.sort_by(|a, b| b.modified.cmp(&a.modified));
    entries.truncate(60);
    Ok(entries)
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
            .map(|e| VIDEO_EXTS.contains(&e.to_lowercase().as_str()))
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

/// Lazily render a poster-frame thumbnail for a Clip into the app cache dir.
/// Keyed by path + mtime so it regenerates if the file changes; returns the
/// cached JPG path (which the UI loads via `convertFileSrc`).
#[tauri::command]
async fn clip_thumbnail(app: AppHandle, path: String) -> Result<String, String> {
    use std::hash::{Hash, Hasher};

    let meta = std::fs::metadata(&path).map_err(|e| e.to_string())?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    mtime.hash(&mut hasher);
    let key = format!("{:016x}", hasher.finish());

    let dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join("thumbs");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let out = dir.join(format!("{key}.jpg"));
    let out_str = out.to_string_lossy().to_string();
    if out.exists() {
        return Ok(out_str);
    }

    // Seek to ~10% in for a representative frame (a few seconds minimum).
    let dur = probe_duration(&app, &path).await;
    let seek = if dur > 0.0 { (dur * 0.1).clamp(1.0, dur - 0.1) } else { 1.0 };

    let args = vec![
        "-ss".into(),
        format!("{seek:.3}"),
        "-i".into(),
        path.clone(),
        "-frames:v".into(),
        "1".into(),
        "-vf".into(),
        "scale=480:-2".into(),
        "-q:v".into(),
        "4".into(),
        "-y".into(),
        out_str.clone(),
    ];
    let output = run_ffmpeg(&app, args).await?;
    if !output.status.success() || !out.exists() {
        return Err(format!(
            "thumbnail failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(out_str)
}

fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_config_dir()
        .map_err(|e| e.to_string())?
        .join("settings.json"))
}

#[tauri::command]
fn get_settings(app: AppHandle) -> Result<Settings, String> {
    let p = settings_path(&app)?;
    if p.exists() {
        let raw = std::fs::read_to_string(&p).map_err(|e| e.to_string())?;
        return serde_json::from_str(&raw).map_err(|e| e.to_string());
    }
    let watched_folder = app
        .path()
        .video_dir()
        .ok()
        .map(|p| p.to_string_lossy().to_string());
    Ok(Settings { watched_folder })
}

#[tauri::command]
fn set_settings(app: AppHandle, settings: Settings) -> Result<(), String> {
    let p = settings_path(&app)?;
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    std::fs::write(&p, json).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            probe_clip,
            trim_clip,
            compress_clip,
            list_recent_clips,
            clip_thumbnail,
            get_settings,
            set_settings
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_stem_falls_back_when_empty_or_blank() {
        assert_eq!(clean_stem(None, "def"), "def");
        assert_eq!(clean_stem(Some("   "), "def"), "def");
        assert_eq!(clean_stem(Some(""), "def"), "def");
    }

    #[test]
    fn clean_stem_strips_path_extension_and_illegal_chars() {
        // directory components dropped
        assert_eq!(clean_stem(Some("C:/evil/clip"), "def"), "clip");
        assert_eq!(clean_stem(Some("../../clip"), "def"), "clip");
        // trailing extension stripped
        assert_eq!(clean_stem(Some("clip.mp4"), "def"), "clip");
        // illegal filename chars removed
        assert_eq!(clean_stem(Some("a<b>c:d"), "def"), "abcd");
        // a name that is only illegal chars collapses to the default
        assert_eq!(clean_stem(Some("///"), "def"), "def");
    }

    #[test]
    fn resolve_output_uses_bare_name_when_free() {
        let dir = std::env::temp_dir().join("klipt_test_resolve_free");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let out = resolve_output(&dir, "clip", "mp4");
        assert_eq!(out, dir.join("clip.mp4"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn resolve_output_avoids_collisions() {
        let dir = std::env::temp_dir().join("klipt_test_resolve_collide");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("clip.mp4"), b"x").unwrap();
        std::fs::write(dir.join("clip_2.mp4"), b"x").unwrap();
        let out = resolve_output(&dir, "clip", "mp4");
        assert_eq!(out, dir.join("clip_3.mp4"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn size_target_bitrate_subtracts_audio_and_scales() {
        // 25 MB over 10 s: total = 25*1024*8/10 * 0.95 = 19456 kbps;
        // video = 19456 - 128 = 19328 kbps.
        let (v, maxrate, bufsize) = size_target_bitrate(25.0, 10.0, 128.0);
        assert!((v - 19328.0).abs() < 1.0, "v_kbps was {v}");
        assert!((maxrate - v * 1.45).abs() < 1.0);
        assert!((bufsize - v * 2.0).abs() < 1.0);
    }

    #[test]
    fn size_target_bitrate_clamps_floor_and_target() {
        // Tiny target over a long clip hits the 300 kbps video floor.
        let (v, _, _) = size_target_bitrate(1.0, 600.0, 128.0);
        assert_eq!(v, 300.0);
        // target below 1 MB is clamped up to 1 MB internally.
        let (a, _, _) = size_target_bitrate(0.0, 10.0, 128.0);
        let (b, _, _) = size_target_bitrate(1.0, 10.0, 128.0);
        assert_eq!(a, b);
    }
}
