//! The Tauri command surface the frontend invokes: probing, the Trim / Compress
//! / GIF export paths, lazy thumbnail / waveform / filmstrip generation, and the
//! delete / restore / rename file operations. The heavy lifting lives in the
//! `ffmpeg`, `naming`, `media`, and `settings` modules; this layer wires them to
//! the bundled sidecar and the app's cache/config dirs.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::ffmpeg::{ffmpeg_probe, parse_ffmpeg_probe, run_ffmpeg, run_ffmpeg_progress};
use crate::media::{
    filmstrip_args, gif_args, nvenc_unavailable, peaks, quality_scale_filter, size_target_bitrate,
    waveform_args, FilmstripOpts, GifOpts, WaveformOpts, NVENC_DISABLED,
};
use crate::naming::{prepare_output, rename_target};
use crate::settings::{ensure_output_dir, read_settings};

/// Metadata about a source Clip, used to drive the timeline.
#[derive(Serialize)]
pub(crate) struct ClipInfo {
    duration: f64,
    width: u32,
    height: u32,
    /// Frames per second of the first video stream (0.0 if unknown). Used by
    /// the editor for frame-accurate playhead stepping.
    fps: f64,
    size_bytes: u64,
}

/// Result of a Trim or Compress.
#[derive(Serialize)]
pub(crate) struct TrimResult {
    path: String,
    size_bytes: u64,
    encoder: Option<String>,
}

/// Result of a lazy library-card thumbnail render. `healthy` is `false` when
/// ffmpeg couldn't read a valid duration from the file's banner — a
/// header-corruption signal (e.g. a crashed ShadowPlay recording). Carrying it
/// here lets the grid flag bad clips off the *same* ffmpeg run that makes the
/// poster, instead of a second `probe_clip` process per card.
#[derive(Serialize)]
pub(crate) struct ThumbResult {
    path: String,
    healthy: bool,
    /// Clip duration in seconds parsed from the same ffmpeg banner used for
    /// `healthy` (0.0 when unknown — e.g. a cache hit that ran no ffmpeg). The
    /// frontend forwards it to `clip_filmstrip` so the hover-scrub render skips
    /// a redundant probe spawn.
    duration: f64,
}

fn file_size(p: &str) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

/// Size of a file we just wrote and expect to exist (a completed export). Unlike
/// [`file_size`], this surfaces a stat failure instead of silently reporting 0 —
/// after a successful ffmpeg run a 0 here would be a misleading number on the
/// result toast, not a legitimately empty file.
fn file_size_checked(p: &str) -> Result<u64, String> {
    std::fs::metadata(p)
        .map(|m| m.len())
        .map_err(|e| e.to_string())
}

/// Remove the x264 two-pass scratch files for `passlog`. ffmpeg names them
/// `<passlog>-<stream>.log` (+ `.mbtree`), one set per stream, so we match the
/// prefix rather than assuming stream index 0. Best-effort: a stray scratch file
/// is harmless, so any failure is ignored.
fn cleanup_passlog(passlog: &Path) {
    let (Some(dir), Some(stem)) = (
        passlog.parent(),
        passlog.file_name().and_then(|s| s.to_str()),
    ) else {
        return;
    };
    let prefix = format!("{stem}-");
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        if let Some(name) = entry.file_name().to_str() {
            if name.starts_with(&prefix)
                && (name.ends_with(".log") || name.ends_with(".log.mbtree"))
            {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

/// A cache key derived from a Clip's path + mtime + an extra discriminator
/// (e.g. bucket count). Regenerates when the file changes. Used by the lazy,
/// mtime-keyed waveform / filmstrip caches (same idea as `clip_thumbnail`).
fn cache_key(path: &str, extra: &str) -> Result<String, String> {
    use std::hash::{Hash, Hasher};
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    mtime.hash(&mut hasher);
    extra.hash(&mut hasher);
    Ok(format!("{:016x}", hasher.finish()))
}

/// Probe a Clip for the duration and dimensions the UI needs, via ffmpeg's
/// `-i` banner. Errors only when nothing could be parsed — a valid video always
/// reports non-zero dimensions, so all-zero means the file is unreadable.
#[tauri::command]
pub(crate) async fn probe_clip(app: AppHandle, path: String) -> Result<ClipInfo, String> {
    let (duration, width, height, fps) = ffmpeg_probe(&app, &path).await;
    if duration == 0.0 && width == 0 && height == 0 {
        return Err("Could not read this clip (ffmpeg could not probe it).".into());
    }
    Ok(ClipInfo {
        duration,
        width,
        height,
        fps,
        size_bytes: file_size(&path),
    })
}

/// Losslessly trim the Region [start, end] out of a Clip via stream-copy.
/// Keeps every video and audio stream; never overwrites an existing file.
#[tauri::command]
pub(crate) async fn trim_clip(
    app: AppHandle,
    path: String,
    start: f64,
    end: f64,
    output_name: Option<String>,
) -> Result<TrimResult, String> {
    let input = PathBuf::from(&path);
    let ext = input.extension().and_then(|s| s.to_str()).unwrap_or("mp4");
    let settings = read_settings(&app);
    let out_dir = ensure_output_dir(&settings)?;
    let out_str = prepare_output(
        &path,
        start,
        end,
        output_name.as_deref(),
        "trim",
        ext,
        out_dir.as_deref(),
        settings.naming_scheme.as_deref(),
    )?;
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
        size_bytes: file_size_checked(&out_str)?,
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
pub(crate) async fn compress_clip(
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
    let settings = read_settings(&app);
    let out_dir = ensure_output_dir(&settings)?;
    let out_str = prepare_output(
        &path,
        start,
        end,
        output_name.as_deref(),
        "small",
        "mp4",
        out_dir.as_deref(),
        settings.naming_scheme.as_deref(),
    )?;

    const AUDIO_KBPS: f64 = 128.0;

    // Quality mode trades size via a resolution preset (a downscale filter),
    // encoding at a fixed high quality. Size mode never scales.
    let scale_filter = if mode == "size" {
        None
    } else {
        quality_scale_filter(quality.as_deref().unwrap_or("source"))
    };

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
            // quality mode: encode at a fixed high quality (CQ/CRF). The chosen
            // resolution preset (applied as a downscale in `build`) is what
            // trades file size against detail.
            if encoder == "h264_nvenc" {
                a.extend([
                    "-preset".into(),
                    "p5".into(),
                    "-rc".into(),
                    "vbr".into(),
                    "-cq".into(),
                    "23".into(),
                    "-b:v".into(),
                    "0".into(),
                ]);
            } else {
                a.extend([
                    "-preset".into(),
                    "medium".into(),
                    "-crf".into(),
                    "20".into(),
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
        if let Some(vf) = &scale_filter {
            args.push("-vf".into());
            args.push(vf.clone());
        }
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

    // Try GPU first (NVENC uses -multipass fullres for size mode — single run),
    // unless an earlier compress this session already proved NVENC unavailable.
    if !NVENC_DISABLED.load(Ordering::Relaxed) {
        // NVENC is a single run (fullres multipass internally) → full bar.
        let nvenc = run_ffmpeg_progress(&app, build("h264_nvenc"), dur, 0.0, 1.0).await?;
        if nvenc.success {
            return Ok(TrimResult {
                size_bytes: file_size_checked(&out_str)?,
                path: out_str,
                encoder: Some("NVENC (GPU)".into()),
            });
        }
        // Only remember the failure when it means NVENC is unsupported here,
        // not for a transient or clip-specific error — otherwise one hiccup
        // would wrongly disable the GPU path for the rest of the session.
        if nvenc_unavailable(&nvenc.stderr) {
            NVENC_DISABLED.store(true, Ordering::Relaxed);
        }
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

        // Two-pass: map pass 1 → 0–50% and pass 2 → 50–100% of the bar.
        let pass1 =
            run_ffmpeg_progress(&app, build_x264_size_pass(1, &passlog), dur, 0.0, 0.5).await?;
        if !pass1.success {
            // Pass 1 wrote the scratch log before failing — clean it up here too,
            // not just on the success path.
            cleanup_passlog(&passlog_path);
            return Err(format!("Compression failed (pass 1): {}", pass1.stderr));
        }

        // Don't `?` pass 2: pass 1 already left scratch files on disk, so clean up
        // before propagating even a spawn error.
        let pass2 =
            run_ffmpeg_progress(&app, build_x264_size_pass(2, &passlog), dur, 0.5, 0.5).await;
        cleanup_passlog(&passlog_path);
        let pass2 = pass2?;

        if pass2.success {
            return Ok(TrimResult {
                size_bytes: file_size_checked(&out_str)?,
                path: out_str,
                encoder: Some("x264 (CPU)".into()),
            });
        }
        return Err(format!("Compression failed (pass 2): {}", pass2.stderr));
    }

    let x264 = run_ffmpeg_progress(&app, build("libx264"), dur, 0.0, 1.0).await?;
    if x264.success {
        return Ok(TrimResult {
            size_bytes: file_size_checked(&out_str)?,
            path: out_str,
            encoder: Some("x264 (CPU)".into()),
        });
    }

    Err(format!("Compression failed: {}", x264.stderr))
}

/// Render the Region to a looping GIF or animated WebP for pasting into chat.
/// A distinct re-encode action (not a lossless Trim); reuses the collision-safe
/// output-path resolution + naming scheme. `format` is "gif" or "webp".
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub(crate) async fn gif_clip(
    app: AppHandle,
    path: String,
    start: f64,
    end: f64,
    output_name: Option<String>,
    format: String,
    fps: Option<u32>,
    width: Option<u32>,
) -> Result<TrimResult, String> {
    let dur = end - start;
    let webp = format == "webp";
    let ext = if webp { "webp" } else { "gif" };
    let settings = read_settings(&app);
    let out_dir = ensure_output_dir(&settings)?;
    let out_str = prepare_output(
        &path,
        start,
        end,
        output_name.as_deref(),
        ext,
        ext,
        out_dir.as_deref(),
        settings.naming_scheme.as_deref(),
    )?;

    let opts = GifOpts {
        fps: fps.unwrap_or(15),
        width: width.unwrap_or(640),
        webp,
    };
    let output = run_ffmpeg(&app, gif_args(&path, start, dur, &opts, &out_str)).await?;
    if !output.status.success() {
        return Err(format!(
            "{} export failed: {}",
            ext.to_uppercase(),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(TrimResult {
        size_bytes: file_size_checked(&out_str)?,
        path: out_str,
        encoder: Some(if webp { "WebP" } else { "GIF" }.into()),
    })
}

/// Lazily render a poster-frame thumbnail for a Clip into the app cache dir.
/// Keyed by path + mtime so it regenerates if the file changes; returns the
/// cached JPG path (which the UI loads via `convertFileSrc`) plus a `healthy`
/// flag derived from the same ffmpeg run (see `ThumbResult`).
#[tauri::command]
pub(crate) async fn clip_thumbnail(app: AppHandle, path: String) -> Result<ThumbResult, String> {
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
        // A cached thumb means this Clip decoded fine when it was first made;
        // treat it as healthy without re-running ffmpeg (the mtime-keyed name
        // already regenerates the cache if the file changed underneath us).
        return Ok(ThumbResult {
            path: out_str,
            healthy: true,
            // No ffmpeg ran on a cache hit, so the duration is unknown; the
            // frontend falls back to clip_filmstrip's own probe.
            duration: 0.0,
        });
    }

    // Grab a single keyframe ~1s in with an input-side `-ss` (jumps the demuxer
    // straight to the nearest keyframe and decodes just that one frame), instead
    // of the old `thumbnail` filter that decoded ~100 frames to vote on the most
    // representative. A 1s offset also clears most intro fades, so the single
    // frame is a fine poster. The `-i` banner ffmpeg prints carries the Clip's
    // Duration, which we parse for the health check — folding what used to be a
    // separate `probe_clip` process into this one run.
    let args = vec![
        "-hide_banner".into(),
        // One decode thread: a 1-frame thumbnail gains nothing from decode
        // parallelism, and up to THUMB_CONCURRENCY of these run at once — so
        // per-process thread buffers are the RAM cost, not a speed win. A
        // decoder option, so it must precede -i.
        "-threads".into(),
        "1".into(),
        "-ss".into(),
        "1".into(),
        "-i".into(),
        path.clone(),
        "-frames:v".into(),
        "1".into(),
        "-vf".into(),
        "scale=480:-2".into(),
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
    // A readable duration means ffmpeg parsed a valid container header; a zero
    // means a truncated / header-corrupt file (a crashed recording) even though
    // a frame still decoded — the case the old standalone probe existed to catch.
    let (duration, _, _, _) = parse_ffmpeg_probe(&String::from_utf8_lossy(&output.stderr));
    Ok(ThumbResult {
        path: out_str,
        healthy: duration > 0.0,
        duration,
    })
}

/// Extract a normalised audio waveform (peaks in `[0, 1]`) for the Timeline.
/// Generated lazily via the bundled FFmpeg sidecar and cached mtime-keyed (like
/// `clip_thumbnail`) so re-opening a Clip is instant. A Clip with no audio (or
/// an unreadable one) yields a flat waveform rather than an error — the waveform
/// is decorative and must never block the editor.
#[tauri::command]
pub(crate) async fn clip_waveform(
    app: AppHandle,
    path: String,
    buckets: Option<usize>,
) -> Result<Vec<f32>, String> {
    let buckets = buckets.unwrap_or(400).clamp(20, 2000);

    // "wf2" bumps the cache when the reduction algorithm changes (peak → RMS).
    let key = cache_key(&path, &format!("wf2_{buckets}"))?;
    let dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join("waveforms");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let cache = dir.join(format!("{key}.json"));
    if let Ok(raw) = std::fs::read_to_string(&cache) {
        if let Ok(v) = serde_json::from_str::<Vec<f32>>(&raw) {
            return Ok(v);
        }
    }

    let opts = WaveformOpts { sample_rate: 4000 };
    let out = run_ffmpeg(&app, waveform_args(&path, &opts)).await?;
    // The waveform is decorative, so a failed decode still yields a flat strip
    // rather than an error. But log a non-zero exit that produced no PCM so a
    // locked/unreadable file is distinguishable from a genuinely silent Clip.
    if !out.status.success() && out.stdout.is_empty() {
        eprintln!(
            "clip_waveform: ffmpeg exited {:?} with no audio for {path}: {}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    // Parse little-endian i16 PCM. Empty stdout (e.g. a silent / audio-less
    // Clip, even on a non-zero exit) reduces to a flat waveform.
    let samples: Vec<i16> = out
        .stdout
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect();
    let data = peaks(&samples, buckets);

    if let Ok(json) = serde_json::to_string(&data) {
        let _ = std::fs::write(&cache, json);
    }
    Ok(data)
}

/// Render a filmstrip sprite (one row of evenly-spaced frames) for thumbnail
/// scrubbing — the Timeline hover-preview and library-card hover both read the
/// same sprite. `cols` frames are sampled across the whole Clip. Generated
/// lazily via the bundled FFmpeg sidecar and cached mtime-keyed (like
/// `clip_thumbnail`); returns the cached JPG path. The frontend maps a hover
/// position to a cell with the pure `frameIndexAt`.
#[tauri::command]
pub(crate) async fn clip_filmstrip(
    app: AppHandle,
    path: String,
    cols: Option<u32>,
    duration: Option<f64>,
) -> Result<String, String> {
    let cols = cols.unwrap_or(16).clamp(4, 64);

    // "fs2" bumps the cache when the sampling changed (per-cell seek midpoints →
    // keyframe-only resample), so stale sprites at the old positions regenerate.
    let key = cache_key(&path, &format!("fs2_{cols}"))?;
    let dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join("filmstrips");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let out = dir.join(format!("{key}.jpg"));
    let out_str = out.to_string_lossy().to_string();
    if out.exists() {
        return Ok(out_str);
    }

    // Duration spaces the samples evenly across the Clip. The editor already
    // probed it (via `probe_clip`) and passes it in, so we skip a redundant
    // ffmpeg banner probe here; only the library-card hover path (no known
    // duration) falls back to probing.
    let duration = match duration {
        Some(d) if d > 0.0 => d,
        _ => ffmpeg_probe(&app, &path).await.0,
    };
    let opts = FilmstripOpts {
        cols,
        frame_width: 160,
        duration,
    };
    let output = run_ffmpeg(&app, filmstrip_args(&path, &opts, &out_str)).await?;
    if !output.status.success() || !out.exists() {
        return Err(format!(
            "filmstrip failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(out_str)
}

/// Send a Clip to the OS recycle bin / trash. Used to discard the source after
/// a successful Trim or Compress when the user opted in. Trashing (rather than a
/// hard delete) keeps the action reversible if they change their mind.
#[tauri::command]
pub(crate) async fn delete_clip(path: String) -> Result<(), String> {
    let p = PathBuf::from(&path);
    if !p.exists() {
        return Err("That file no longer exists.".into());
    }
    // The webview releases the media file handle asynchronously after the
    // frontend detaches the <video> element, so an immediate trash can fail
    // with a Windows "some operations were aborted" error while that handle
    // still lingers. Retry a few times with a short backoff to ride it out;
    // surface the last error only if the file stays locked.
    let mut last = String::new();
    for attempt in 0..8u64 {
        match trash::delete(&p) {
            Ok(()) => return Ok(()),
            Err(e) => last = e.to_string(),
        }
        std::thread::sleep(std::time::Duration::from_millis(100 + attempt * 50));
    }
    Err(last)
}

/// Restore a previously-trashed Clip from the Recycle Bin back to its original
/// location. Backs the "Undo" on the delete-original toast. Windows' trash has
/// no restore-by-path call, so we list the bin, match the item whose original
/// path equals `path` (newest wins if the same path was trashed more than once),
/// and restore just that one.
#[tauri::command]
pub(crate) async fn restore_clip(path: String) -> Result<(), String> {
    use trash::os_limited::{list, restore_all};

    let target = PathBuf::from(&path);
    let items = list().map_err(|e| e.to_string())?;
    // Key each trash entry by (original path, deletion time), then pick purely —
    // keeping the `trash` I/O out of the "newest match wins" selection so it can
    // be unit-tested without touching the real Recycle Bin.
    let keyed: Vec<(PathBuf, i64)> = items
        .iter()
        .map(|it| (it.original_path(), it.time_deleted))
        .collect();
    let idx =
        pick_restore_index(&keyed, &target).ok_or("Couldn't find that clip in the Recycle Bin.")?;
    let newest = items.into_iter().nth(idx).unwrap();
    restore_all([newest]).map_err(|e| e.to_string())
}

/// Pure selection step for [`restore_clip`]: from `(original_path, time_deleted)`
/// trash entries, return the index of the entry whose path equals `target` and
/// was deleted most recently — the copy the user just trashed when the same path
/// has been deleted more than once. Returns `None` when nothing matches.
fn pick_restore_index(entries: &[(PathBuf, i64)], target: &Path) -> Option<usize> {
    entries
        .iter()
        .enumerate()
        .filter(|(_, (p, _))| p == target)
        .max_by_key(|(_, (_, t))| *t)
        .map(|(i, _)| i)
}

/// Rename a Clip in place (same folder, same extension), sanitizing the name and
/// avoiding collisions. Returns the new path so the UI can refresh.
#[tauri::command]
pub(crate) async fn rename_clip(path: String, new_name: String) -> Result<String, String> {
    let target = rename_target(&path, &new_name)?;
    if target == path {
        return Ok(target); // nothing to do
    }
    std::fs::rename(&path, &target).map_err(|e| e.to_string())?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pb(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn pick_restore_index_returns_newest_when_path_trashed_twice() {
        // Same original path trashed at two different times; the later deletion
        // (the copy the user just trashed) must win.
        let entries = vec![
            (pb("C:/clips/a.mp4"), 100),
            (pb("C:/clips/b.mp4"), 150),
            (pb("C:/clips/a.mp4"), 200),
        ];
        let idx = pick_restore_index(&entries, Path::new("C:/clips/a.mp4")).unwrap();
        assert_eq!(
            idx, 2,
            "should pick the most recently deleted matching entry"
        );
    }

    #[test]
    fn pick_restore_index_matches_only_the_target_path() {
        let entries = vec![(pb("C:/clips/a.mp4"), 100), (pb("C:/clips/b.mp4"), 200)];
        let idx = pick_restore_index(&entries, Path::new("C:/clips/a.mp4")).unwrap();
        assert_eq!(idx, 0);
    }

    #[test]
    fn pick_restore_index_none_when_no_match() {
        let entries = vec![(pb("C:/clips/a.mp4"), 100)];
        assert!(pick_restore_index(&entries, Path::new("C:/clips/missing.mp4")).is_none());
        assert!(pick_restore_index(&[], Path::new("C:/clips/a.mp4")).is_none());
    }

    #[test]
    fn cleanup_passlog_removes_every_stream_log_and_keeps_others() {
        // ffmpeg can write a log per stream (`-0.log`, `-1.log`, …) plus mbtree
        // sidecars; cleanup must glob the prefix, not just stream 0, and leave
        // unrelated files alone.
        let dir = std::env::temp_dir().join(format!("klipt_test_passlog_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let passlog = dir.join("klipt-passlog");
        for f in [
            "klipt-passlog-0.log",
            "klipt-passlog-0.log.mbtree",
            "klipt-passlog-1.log",
            "klipt-passlog-1.log.mbtree",
        ] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        // Files that must survive: a different prefix, and the export itself.
        std::fs::write(dir.join("other-0.log"), b"x").unwrap();
        std::fs::write(dir.join("klipt-passlog.mp4"), b"x").unwrap();

        cleanup_passlog(&passlog);

        assert!(!dir.join("klipt-passlog-0.log").exists());
        assert!(!dir.join("klipt-passlog-0.log.mbtree").exists());
        assert!(!dir.join("klipt-passlog-1.log").exists());
        assert!(!dir.join("klipt-passlog-1.log.mbtree").exists());
        assert!(dir.join("other-0.log").exists(), "unrelated log kept");
        assert!(dir.join("klipt-passlog.mp4").exists(), "export kept");

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
