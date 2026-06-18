use std::cmp::Reverse;
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

/// Validate the Region and resolve a collision-free output path next to the
/// source Clip. `default_suffix` is appended to the source stem when the user
/// gave no name (e.g. "_trim", "_small"); `out_ext` is the output extension.
// The negated comparison is deliberate: `!(end > start)` also rejects a NaN
// endpoint (NaN > x is false), whereas `end <= start` would let NaN through.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
fn prepare_output(
    path: &str,
    start: f64,
    end: f64,
    output_name: Option<&str>,
    default_suffix: &str,
    out_ext: &str,
) -> Result<String, String> {
    if !(end > start) {
        return Err("End point must be after the start point.".into());
    }
    let input = PathBuf::from(path);
    let parent = input
        .parent()
        .ok_or("Could not resolve the clip's folder.")?;
    let src_stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or("Could not read the clip's name.")?;
    let stem = clean_stem(output_name, &format!("{src_stem}{default_suffix}"));
    let out_path = resolve_output(parent, &stem, out_ext);
    Ok(out_path.to_string_lossy().to_string())
}

/// Compute the video bitrate ladder (kbps) for a size-targeted encode.
/// Returns (video_kbps, maxrate_kbps, bufsize_kbps). `dur` is the Region length
/// in seconds and must be > 0 (callers guarantee this via the end > start guard).
fn size_target_bitrate(target_mb: f64, dur: f64, audio_kbps: f64) -> (f64, f64, f64) {
    let target = target_mb.max(1.0);
    // Aim under target to leave headroom for container overhead + VBR variance.
    let total_kbps = (target * 1024.0 * 8.0 / dur) * 0.90;
    let v_kbps = (total_kbps - audio_kbps).max(300.0);
    // Cap the peak close to the average so two-pass can't blow the budget.
    (v_kbps, v_kbps * 1.10, v_kbps * 1.5)
}

fn file_size(p: &str) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

/// Parse ffmpeg's `-i` stderr banner for a Clip's duration (seconds) and the
/// first video stream's pixel dimensions. ffmpeg prints these lines in English
/// regardless of system locale. Any field that can't be found stays 0.
fn parse_ffmpeg_probe(stderr: &str) -> (f64, u32, u32) {
    let mut duration = 0.0;
    let mut width = 0u32;
    let mut height = 0u32;

    // "  Duration: 00:02:34.56, start: 0.000000, bitrate: 8234 kb/s"
    if let Some(idx) = stderr.find("Duration:") {
        let token = stderr[idx + "Duration:".len()..]
            .trim_start()
            .split(',')
            .next()
            .unwrap_or("")
            .trim();
        let parts: Vec<&str> = token.split(':').collect();
        if parts.len() == 3 {
            if let (Ok(h), Ok(m), Ok(s)) = (
                parts[0].parse::<f64>(),
                parts[1].parse::<f64>(),
                parts[2].parse::<f64>(),
            ) {
                duration = h * 3600.0 + m * 60.0 + s;
            }
        }
    }

    // First "Video:" line carries dimensions as a WxH token, e.g.
    // "Stream #0:0: Video: h264 ..., yuv420p, 1920x1080 [SAR 1:1 DAR 16:9], ..."
    // Splitting on space/comma isolates "1920x1080"; the >=16 guard rejects the
    // hex codec tag (e.g. "0x31637661" -> 0) and other stray tokens.
    if let Some(line) = stderr.lines().find(|l| l.contains("Video:")) {
        for tok in line.split([' ', ',']) {
            if let Some((w, h)) = tok.split_once('x') {
                if let (Ok(w), Ok(h)) = (w.parse::<u32>(), h.parse::<u32>()) {
                    if w >= 16 && h >= 16 {
                        width = w;
                        height = h;
                        break;
                    }
                }
            }
        }
    }

    (duration, width, height)
}

/// Probe a Clip's duration (seconds) and first-video-stream dimensions using
/// ffmpeg's `-i` banner. ffmpeg exits non-zero when given no output file but
/// prints the stream info to stderr first, which is what we parse.
/// Best-effort: returns zeros on any failure.
async fn ffmpeg_probe(app: &AppHandle, path: &str) -> (f64, u32, u32) {
    match run_ffmpeg(
        app,
        vec!["-hide_banner".into(), "-i".into(), path.to_string()],
    )
    .await
    {
        Ok(out) => parse_ffmpeg_probe(&String::from_utf8_lossy(&out.stderr)),
        Err(_) => (0.0, 0, 0),
    }
}

// --- commands -------------------------------------------------------------

/// Probe a Clip for the duration and dimensions the UI needs, via ffmpeg's
/// `-i` banner. Errors only when nothing could be parsed — a valid video always
/// reports non-zero dimensions, so all-zero means the file is unreadable.
#[tauri::command]
async fn probe_clip(app: AppHandle, path: String) -> Result<ClipInfo, String> {
    let (duration, width, height) = ffmpeg_probe(&app, &path).await;
    if duration == 0.0 && width == 0 && height == 0 {
        return Err("Could not read this clip (ffmpeg could not probe it).".into());
    }
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
    let input = PathBuf::from(&path);
    let ext = input.extension().and_then(|s| s.to_str()).unwrap_or("mp4");
    let out_str = prepare_output(&path, start, end, output_name.as_deref(), "_trim", ext)?;
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
// Args mirror the frontend `invoke` payload; a parameter struct would couple
// a frontend change for no backend benefit.
#[allow(clippy::too_many_arguments)]
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
    let dur = end - start;
    // Always output mp4 for maximum share compatibility (Discord, browsers).
    let out_str = prepare_output(&path, start, end, output_name.as_deref(), "_small", "mp4")?;

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
                a.extend([
                    "-preset".into(),
                    "p5".into(),
                    "-rc".into(),
                    "vbr".into(),
                    "-multipass".into(),
                    "fullres".into(),
                ]);
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

    // Build args for x264 size-mode pass 1 or pass 2 (two-pass encode).
    // Pass 1 discards output to the null sink; pass 2 writes the real file.
    let build_x264_size_pass = |pass: u8, passlog: &str| -> Vec<String> {
        let target = target_mb.unwrap_or(25.0);
        let (v_kbps, maxrate, bufsize) = size_target_bitrate(target, dur, AUDIO_KBPS);
        let null_sink = if cfg!(windows) { "NUL" } else { "/dev/null" };
        let mut args: Vec<String> = vec![
            "-ss".into(),
            format!("{start}"),
            "-i".into(),
            path.clone(),
            "-t".into(),
            format!("{dur}"),
            "-map".into(),
            "0:v:0".into(),
            "-c:v".into(),
            "libx264".into(),
            "-preset".into(),
            "medium".into(),
            "-b:v".into(),
            format!("{v_kbps:.0}k"),
            "-maxrate".into(),
            format!("{maxrate:.0}k"),
            "-bufsize".into(),
            format!("{bufsize:.0}k"),
            "-pass".into(),
            pass.to_string(),
            "-passlogfile".into(),
            passlog.to_string(),
        ];
        if pass == 1 {
            // Pass 1: discard audio, write to null sink.
            args.extend(["-an".into(), "-f".into(), "null".into(), null_sink.into()]);
        } else {
            // Pass 2: map optional audio, write the real output.
            args.extend([
                "-map".into(),
                "0:a?".into(),
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
        }
        args
    };

    // Try GPU first (NVENC uses -multipass fullres for size mode — single run).
    let nvenc = run_ffmpeg(&app, build("h264_nvenc")).await?;
    if nvenc.status.success() {
        return Ok(TrimResult {
            size_bytes: file_size(&out_str),
            path: out_str,
            encoder: Some("NVENC (GPU)".into()),
        });
    }

    // Fall back to CPU x264. Use two-pass for size mode so the output stays
    // under the requested cap; quality mode uses the single-pass CRF path.
    if mode == "size" {
        let passlog_path = app
            .path()
            .app_cache_dir()
            .map_err(|e| e.to_string())?
            .join("klipt-passlog");
        std::fs::create_dir_all(passlog_path.parent().ok_or("invalid cache dir")?)
            .map_err(|e| e.to_string())?;
        let passlog = passlog_path.to_string_lossy().to_string();

        let pass1 = run_ffmpeg(&app, build_x264_size_pass(1, &passlog)).await?;
        if !pass1.status.success() {
            return Err(format!(
                "Compression failed (pass 1): {}",
                String::from_utf8_lossy(&pass1.stderr)
            ));
        }

        let pass2 = run_ffmpeg(&app, build_x264_size_pass(2, &passlog)).await?;

        // Best-effort cleanup of passlog scratch files.
        let _ = std::fs::remove_file(format!("{passlog}-0.log"));
        let _ = std::fs::remove_file(format!("{passlog}-0.log.mbtree"));

        if pass2.status.success() {
            return Ok(TrimResult {
                size_bytes: file_size(&out_str),
                path: out_str,
                encoder: Some("x264 (CPU)".into()),
            });
        }
        return Err(format!(
            "Compression failed (pass 2): {}",
            String::from_utf8_lossy(&pass2.stderr)
        ));
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
/// newest first. Declared `async` so Tauri runs the directory walk on its async
/// runtime rather than the main thread, keeping the UI responsive during the scan.
#[tauri::command]
async fn list_recent_clips(folder: String) -> Result<Vec<ClipEntry>, String> {
    let mut entries = Vec::new();
    collect_clips(&PathBuf::from(&folder), 0, &mut entries);
    entries.sort_by_key(|e| Reverse(e.modified));
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

    // Pick a representative frame with ffmpeg's `thumbnail` filter, which
    // scans a window of frames and returns the most representative one. This
    // needs no duration, so we avoid a second ffmpeg probe per clip.
    let args = vec![
        "-i".into(),
        path.clone(),
        "-frames:v".into(),
        "1".into(),
        "-vf".into(),
        "thumbnail,scale=480:-2".into(),
        "-an".into(),
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
        // 25 MB over 10 s: total = 25*1024*8/10 * 0.90 = 18432 kbps;
        // video = 18432 - 128 = 18304 kbps.
        let (v, maxrate, bufsize) = size_target_bitrate(25.0, 10.0, 128.0);
        assert!((v - 18304.0).abs() < 1.0, "v_kbps was {v}");
        assert!((maxrate - v * 1.10).abs() < 1.0);
        assert!((bufsize - v * 1.5).abs() < 1.0);
    }

    #[test]
    fn size_target_bitrate_peak_stays_near_average() {
        let (v, maxrate, _) = size_target_bitrate(10.0, 30.0, 128.0);
        assert!(
            maxrate <= v * 1.2,
            "maxrate {maxrate} too far above avg {v}"
        );
    }

    #[test]
    fn prepare_output_rejects_non_positive_region() {
        assert!(prepare_output("/tmp/a.mp4", 5.0, 5.0, None, "_trim", "mp4").is_err());
        assert!(prepare_output("/tmp/a.mp4", 5.0, 4.0, None, "_trim", "mp4").is_err());
    }

    #[test]
    fn prepare_output_builds_default_suffix_path() {
        let dir = std::env::temp_dir().join("klipt_test_prepare");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("clip.mp4");
        std::fs::write(&src, b"x").unwrap();
        let out = prepare_output(&src.to_string_lossy(), 0.0, 2.0, None, "_trim", "mp4").unwrap();
        assert!(out.ends_with("clip_trim.mp4"), "got {out}");
        std::fs::remove_dir_all(&dir).unwrap();
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

    #[test]
    fn parse_ffmpeg_probe_reads_duration_and_dimensions() {
        let stderr = "\
Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'clip.mp4':
  Metadata:
    major_brand     : isom
  Duration: 00:02:34.56, start: 0.000000, bitrate: 8234 kb/s
  Stream #0:0[0x1](und): Video: h264 (High) (avc1 / 0x31637661), yuv420p(tv), 1920x1080 [SAR 1:1 DAR 16:9], 8000 kb/s, 60 fps
  Stream #0:1[0x2](und): Audio: aac (LC) (mp4a / 0x6134706D), 48000 Hz, stereo, fltp, 234 kb/s
";
        let (d, w, h) = parse_ffmpeg_probe(stderr);
        assert!((d - 154.56).abs() < 0.01, "duration was {d}");
        assert_eq!((w, h), (1920, 1080));
    }

    #[test]
    fn parse_ffmpeg_probe_handles_missing_or_na_fields() {
        // No Duration / no Video line -> all zeros, no panic.
        assert_eq!(parse_ffmpeg_probe("garbage with no fields"), (0.0, 0, 0));
        // Duration N/A parses to 0 but dimensions still read.
        let s = "  Duration: N/A, bitrate: N/A\n  Stream #0:0: Video: h264, 1280x720, 30 fps\n";
        let (d, w, h) = parse_ffmpeg_probe(s);
        assert_eq!(d, 0.0);
        assert_eq!((w, h), (1280, 720));
    }
}
