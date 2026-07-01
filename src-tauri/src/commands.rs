//! The Tauri command surface the frontend invokes: probing, the Trim / Compress
//! / GIF export paths, lazy thumbnail / waveform / filmstrip generation, and the
//! delete / restore / rename file operations. The heavy lifting lives in the
//! `ffmpeg`, `naming`, `media`, and `settings` modules; this layer wires them to
//! the bundled sidecar and the app's cache/config dirs.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::ffmpeg::{
    ffmpeg_probe, parse_ffmpeg_probe, run_ffmpeg, run_ffmpeg_checked, run_ffmpeg_progress,
};
use crate::libav::{filmstrip_libav, LIBAV_DISABLED};
use crate::media::{
    atempo_chain, audio_args, compose_vf_speed, crop_filter, cuda_unavailable, filmstrip_args,
    gif_args, input_segment, input_segment_out, nvenc_unavailable, peaks, quality_scale_filter,
    size_target_bitrate, speed_setpts_filter, waveform_args, FilmstripOpts, GifOpts, WaveformOpts,
    NVDEC_DISABLED, NVENC_DISABLED,
};
use crate::naming::{prepare_output, rename_target};
use crate::settings::{ensure_output_dir, read_settings};

/// Metadata about a source Clip, used to drive the timeline. `Deserialize` so it
/// round-trips through the on-disk probe cache (see `probe_cache_path`).
#[derive(Serialize, Deserialize)]
pub(crate) struct ClipInfo {
    duration: f64,
    width: u32,
    height: u32,
    /// Frames per second of the first video stream (0.0 if unknown). Used by
    /// the editor for frame-accurate playhead stepping.
    fps: f64,
    size_bytes: u64,
}

/// A spatial-crop rectangle in *source* pixels (top-left origin). Sent by the
/// editor's crop overlay; turned into an ffmpeg `crop` filter. Cropping forces a
/// re-encode (incompatible with the lossless `-c copy` Trim, ADR 0002), so this
/// only ever rides on `compress_clip`. The frontend clamps it in-bounds and to
/// even dimensions (yuv420p requires even w/h); the backend trusts those.
#[derive(serde::Deserialize, Clone, Copy)]
pub(crate) struct CropRect {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
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

/// Monotonic discriminator for the two-pass scratch-log path. Combined with the
/// process id it makes each compress's `-passlogfile` unique, so concurrent
/// size-mode encodes (even across two app instances) can't read or delete each
/// other's stats — and `cleanup_passlog`'s prefix glob only matches this run.
static PASSLOG_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

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

/// A file's mtime as whole seconds since the epoch (0 if unavailable) — the
/// discriminator the lazy caches use to regenerate when a Clip changes.
fn mtime_secs(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A cache key derived from a Clip's path + mtime + an extra discriminator
/// (e.g. bucket count). Regenerates when the file changes. Used by all three
/// lazy, mtime-keyed caches (thumbnail / waveform / filmstrip).
fn cache_key(path: &str, extra: &str) -> Result<String, String> {
    use std::hash::{Hash, Hasher};
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    mtime_secs(&meta).hash(&mut hasher);
    extra.hash(&mut hasher);
    Ok(format!("{:016x}", hasher.finish()))
}

/// Resolve `<app_cache>/<subdir>/<key>.<ext>`, creating the subdir. The shared
/// first half of every lazy-render command (thumbnail / waveform / filmstrip);
/// each then either returns the cached file or generates it.
fn cache_path(app: &AppHandle, subdir: &str, key: &str, ext: &str) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join(subdir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join(format!("{key}.{ext}")))
}

/// Disk path for a Clip's cached probe — a tiny JSON sidecar (duration /
/// dimensions / fps / size) in the same mtime-keyed scheme as the thumbnail /
/// waveform / filmstrip caches, so it regenerates when the file changes. Lets
/// `probe_clip` skip an ffmpeg banner spawn (~140 ms of pure overhead on the
/// blocking clip-open path) once any path has probed the Clip — the grid
/// thumbnail warms it for free, since it parses the same banner anyway.
fn probe_cache_path(app: &AppHandle, path: &str) -> Result<PathBuf, String> {
    let key = cache_key(path, "probe1")?;
    cache_path(app, "probes", &key, "json")
}

/// Read a cached `ClipInfo` for `path`, or None if it's absent or unparseable
/// (a stale/corrupt entry just falls through to a fresh probe).
fn read_probe_cache(app: &AppHandle, path: &str) -> Option<ClipInfo> {
    let p = probe_cache_path(app, path).ok()?;
    let raw = std::fs::read_to_string(&p).ok()?;
    serde_json::from_str::<ClipInfo>(&raw).ok()
}

/// Persist a complete `ClipInfo` to the probe cache. Best-effort: a write
/// failure just means the next probe re-runs ffmpeg. Callers should only cache a
/// complete probe (non-zero duration + dimensions), never a partial/failed one.
fn write_probe_cache(app: &AppHandle, path: &str, info: &ClipInfo) {
    if let Ok(p) = probe_cache_path(app, path) {
        if let Ok(json) = serde_json::to_string(info) {
            let _ = std::fs::write(p, json);
        }
    }
}

/// Probe a Clip for the duration and dimensions the UI needs, via ffmpeg's
/// `-i` banner. Errors only when nothing could be parsed — a valid video always
/// reports non-zero dimensions, so all-zero means the file is unreadable.
#[tauri::command]
pub(crate) async fn probe_clip(app: AppHandle, path: String) -> Result<ClipInfo, String> {
    // A prior probe — or the grid thumbnail, which parses the same banner for
    // free — may have cached this Clip's metadata; reuse it to skip the ffmpeg
    // banner spawn that otherwise blocks the editor from opening.
    if let Some(info) = read_probe_cache(&app, &path) {
        return Ok(info);
    }
    let (duration, width, height, fps) = ffmpeg_probe(&app, &path).await;
    if duration == 0.0 && width == 0 && height == 0 {
        return Err("Could not read this clip (ffmpeg could not probe it).".into());
    }
    let info = ClipInfo {
        duration,
        width,
        height,
        fps,
        size_bytes: file_size(&path),
    };
    write_probe_cache(&app, &path, &info);
    Ok(info)
}

/// Losslessly trim the Region [start, end] out of a Clip via stream-copy.
/// Keeps every video stream (and every audio stream when `include_audio`);
/// never overwrites an existing file.
#[tauri::command]
pub(crate) async fn trim_clip(
    app: AppHandle,
    path: String,
    start: f64,
    end: f64,
    output_name: Option<String>,
    include_audio: bool,
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

    // Shared seek/input/duration/map prefix (0:v? keeps all video; audio mapped
    // only when kept), then -c copy (no re-encode). Dropping audio = omitting the
    // 0:a? map, which is lossless and correct alongside -c copy.
    let mut args = input_segment(&path, start, dur, "0:v?", include_audio);
    args.extend([
        "-c".into(),
        "copy".into(),
        "-avoid_negative_ts".into(),
        "make_zero".into(),
        "-y".into(),
        out_str.clone(),
    ]);

    run_ffmpeg_checked(&app, args, "Trim", Some(Path::new(&out_str))).await?;

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
    include_audio: bool,
    crop: Option<CropRect>,
    speed: f64,
) -> Result<TrimResult, String> {
    let dur = end - start;
    // A speed change retimes the output: it plays back over `out_dur` seconds
    // while still reading the same source region `[start, start+dur]`. Every
    // output-duration concern (the `-t` cap, the size budget, the progress bar)
    // must use `out_dur`; `dur` stays for the source read. At 1x they're equal.
    let out_dur = dur / speed;
    // Audio is time-stretched to match (pitch preserved); None at 1x. Present
    // here forces an audio re-encode, which compress already does anyway.
    let atempo = atempo_chain(speed);
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
    // When audio is dropped, give its share of the byte budget back to video.
    let audio_kbps = if include_audio { AUDIO_KBPS } else { 0.0 };

    // Quality mode trades size via a resolution preset (a downscale filter),
    // encoding at a fixed high quality. Size mode never scales.
    let scale_filter = if mode == "size" {
        None
    } else {
        quality_scale_filter(quality.as_deref().unwrap_or("source"))
    };
    // Crop (if any) applies in BOTH modes and must precede scale in the single
    // allowed -vf. Computed once here so every encoder branch shares it.
    let vf = compose_vf_speed(
        crop_filter(crop.map(|c| (c.x, c.y, c.w, c.h))),
        scale_filter,
        speed_setpts_filter(speed),
    );

    // Build the encoder-specific video args for a given encoder.
    let video_args = |encoder: &str| -> Vec<String> {
        let mut a: Vec<String> = Vec::new();
        a.push("-c:v".into());
        a.push(encoder.into());
        if mode == "size" {
            let target = target_mb.unwrap_or(25.0);
            let (v_kbps, maxrate, bufsize) = size_target_bitrate(target, out_dur, audio_kbps);
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
        let mut args = input_segment_out(&path, start, out_dur, "0:v:0", include_audio);
        if let Some(vf) = &vf {
            args.push("-vf".into());
            args.push(vf.clone());
        }
        args.extend(video_args(encoder));
        args.extend(["-pix_fmt".into(), "yuv420p".into()]);
        if include_audio {
            if let Some(af) = &atempo {
                args.push("-filter:a".into());
                args.push(af.clone());
            }
            args.extend([
                "-c:a".into(),
                "aac".into(),
                "-b:a".into(),
                format!("{AUDIO_KBPS:.0}k"),
            ]);
        } else {
            args.push("-an".into());
        }
        args.extend([
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
        let (v_kbps, maxrate, bufsize) = size_target_bitrate(target, out_dur, audio_kbps);
        let null_sink = if cfg!(windows) { "NUL" } else { "/dev/null" };
        // Video-only base; the crop/scale/setpts -vf (when present) must be
        // applied identically in BOTH passes or the pass-1 stats won't match
        // pass 2. (Audio's atempo is pass-2 only — pass 1 is -an.)
        let mut args = input_segment_out(&path, start, out_dur, "0:v:0", false);
        if let Some(vf) = &vf {
            args.push("-vf".into());
            args.push(vf.clone());
        }
        args.extend([
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
        ]);
        if pass == 1 {
            // Pass 1: discard audio, write to null sink.
            args.extend(["-an".into(), "-f".into(), "null".into(), null_sink.into()]);
        } else {
            // Pass 2: write the real output, mapping audio only when kept.
            if include_audio {
                args.extend(["-map".into(), "0:a?".into()]);
            }
            args.extend(["-pix_fmt".into(), "yuv420p".into()]);
            if include_audio {
                if let Some(af) = &atempo {
                    args.push("-filter:a".into());
                    args.push(af.clone());
                }
                args.extend([
                    "-c:a".into(),
                    "aac".into(),
                    "-b:a".into(),
                    format!("{AUDIO_KBPS:.0}k"),
                ]);
            } else {
                args.push("-an".into());
            }
            args.extend([
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
        let nvenc = run_ffmpeg_progress(&app, build("h264_nvenc"), out_dur, 0.0, 1.0).await?;
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
        let seq = PASSLOG_SEQ.fetch_add(1, Ordering::Relaxed);
        let passlog_path = app
            .path()
            .app_cache_dir()
            .map_err(|e| e.to_string())?
            .join(format!("klipt-passlog-{}-{}", std::process::id(), seq));
        std::fs::create_dir_all(passlog_path.parent().ok_or("invalid cache dir")?)
            .map_err(|e| e.to_string())?;
        let passlog = passlog_path.to_string_lossy().to_string();

        // Two-pass: map pass 1 → 0–50% and pass 2 → 50–100% of the bar.
        let pass1 =
            run_ffmpeg_progress(&app, build_x264_size_pass(1, &passlog), out_dur, 0.0, 0.5).await?;
        if !pass1.success {
            // Pass 1 wrote the scratch log before failing — clean it up here too,
            // not just on the success path. A failed GPU attempt may also have
            // left a partial output; drop it so it doesn't linger / bump names.
            cleanup_passlog(&passlog_path);
            let _ = std::fs::remove_file(&out_str);
            return Err(format!("Compression failed (pass 1): {}", pass1.stderr));
        }

        // Don't `?` pass 2: pass 1 already left scratch files on disk, so clean up
        // before propagating even a spawn error.
        let pass2 =
            run_ffmpeg_progress(&app, build_x264_size_pass(2, &passlog), out_dur, 0.5, 0.5).await;
        cleanup_passlog(&passlog_path);
        let pass2 = pass2?;

        if pass2.success {
            return Ok(TrimResult {
                size_bytes: file_size_checked(&out_str)?,
                path: out_str,
                encoder: Some("x264 (CPU)".into()),
            });
        }
        let _ = std::fs::remove_file(&out_str);
        return Err(format!("Compression failed (pass 2): {}", pass2.stderr));
    }

    let x264 = run_ffmpeg_progress(&app, build("libx264"), out_dur, 0.0, 1.0).await?;
    if x264.success {
        return Ok(TrimResult {
            size_bytes: file_size_checked(&out_str)?,
            path: out_str,
            encoder: Some("x264 (CPU)".into()),
        });
    }

    let _ = std::fs::remove_file(&out_str);
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
    speed: f64,
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
    run_ffmpeg_checked(
        &app,
        gif_args(&path, start, dur, speed, &opts, &out_str),
        &format!("{} export", ext.to_uppercase()),
        Some(Path::new(&out_str)),
    )
    .await?;
    Ok(TrimResult {
        size_bytes: file_size_checked(&out_str)?,
        path: out_str,
        encoder: Some(if webp { "WebP" } else { "GIF" }.into()),
    })
}

/// Export just the Region's audio as a standalone file. A distinct re-encode
/// action: M4A stream-copies the source AAC (lossless, instant), falling back to
/// an AAC re-encode if the source isn't AAC; MP3 always re-encodes via
/// libmp3lame. `format` is "m4a" or "mp3". Reuses the collision-safe naming.
#[tauri::command]
pub(crate) async fn audio_clip(
    app: AppHandle,
    path: String,
    start: f64,
    end: f64,
    output_name: Option<String>,
    format: String,
    speed: f64,
) -> Result<TrimResult, String> {
    let mp3 = format == "mp3";
    let ext = if mp3 { "mp3" } else { "m4a" };
    let settings = read_settings(&app);
    let out_dir = ensure_output_dir(&settings)?;
    let out_str = prepare_output(
        &path,
        start,
        end,
        output_name.as_deref(),
        "audio",
        ext,
        out_dir.as_deref(),
        settings.naming_scheme.as_deref(),
    )?;
    let dur = end - start;

    // M4A tries a lossless stream-copy first, then an AAC re-encode if the source
    // codec can't be copied into m4a. MP3 has the single libmp3lame path. A speed
    // change rules out stream-copy (atempo must re-encode), so skip the copy try.
    let can_copy = (speed - 1.0).abs() < 1e-6;
    let attempts: &[bool] = if mp3 || !can_copy {
        &[false]
    } else {
        &[true, false]
    };
    let mut last_err = String::new();
    for &copy in attempts {
        match run_ffmpeg_checked(
            &app,
            audio_args(&path, start, dur, speed, mp3, copy, &out_str),
            "Audio export",
            Some(Path::new(&out_str)),
        )
        .await
        {
            Ok(_) => {
                return Ok(TrimResult {
                    size_bytes: file_size_checked(&out_str)?,
                    path: out_str,
                    encoder: Some(if mp3 { "MP3" } else { "AAC (M4A)" }.into()),
                })
            }
            Err(e) => last_err = e,
        }
    }
    Err(last_err)
}

/// The per-input flags for a poster-frame grab seeking to `seek` seconds. An
/// input-side `-ss` jumps the demuxer to the nearest keyframe; `-skip_frame
/// nokey` then makes the decoder emit *only* keyframes, so it decodes exactly one
/// intra-frame instead of also decoding the inter-frames between that keyframe
/// and the precise `-ss` instant (~2.5x faster on a 1080p/1440p Clip — measured —
/// and the same keyframe-only trick `filmstrip_args` uses). The poster becomes
/// the keyframe at/after the seek rather than the exact-time frame: invisible for
/// a decorative thumbnail, and a clean intra-frame actually looks better. All of
/// these are decoder/input options, so they precede `-i`. `-threads 1` caps the
/// per-process footprint — a 1-frame decode gains nothing from decode threads and
/// several of these run at once. Shared by the single and batch thumbnail paths
/// so their frames are byte-identical.
fn thumb_input_args(path: &str, seek: &str) -> Vec<String> {
    vec![
        "-threads".into(),
        "1".into(),
        "-skip_frame".into(),
        "nokey".into(),
        "-ss".into(),
        seek.into(),
        "-i".into(),
        path.to_string(),
    ]
}

/// The output flags that turn one mapped input into a single 480-wide poster JPG
/// at `out_str`. `map` selects the input stream for the batch path (`"0:v"`,
/// `"1:v"`, …); `None` omits `-map` for the single-input command (ffmpeg auto-
/// picks the lone video stream). Shared so single and batch posters match.
fn thumb_output_args(out_str: &str, map: Option<&str>) -> Vec<String> {
    let mut a = Vec::new();
    if let Some(m) = map {
        a.push("-map".into());
        a.push(m.to_string());
    }
    a.extend([
        "-frames:v".into(),
        "1".into(),
        "-vf".into(),
        "scale=480:-2".into(),
        "-an".into(),
        "-q:v".into(),
        "4".into(),
        "-y".into(),
        out_str.to_string(),
    ]);
    a
}

/// Derive `(duration, healthy)` from a thumbnail run's `-i` banner and warm the
/// probe cache when the probe is complete. A readable duration means ffmpeg
/// parsed a valid container header; a zero means a truncated / header-corrupt
/// file (a crashed recording) even though a frame still decoded — the corruption
/// the health flag exists to catch. The same banner is exactly what `probe_clip`
/// needs, so caching it lets opening this Clip from the grid skip a probe spawn.
/// Shared by the single and batch thumbnail commands so both flag corruption and
/// feed `probe_clip` identically.
fn finalize_thumb_banner(app: &AppHandle, path: &str, banner: &str) -> (f64, bool) {
    let (duration, width, height, fps) = parse_ffmpeg_probe(banner);
    if duration > 0.0 && width > 0 && height > 0 {
        write_probe_cache(
            app,
            path,
            &ClipInfo {
                duration,
                width,
                height,
                fps,
                size_bytes: file_size(path),
            },
        );
    }
    (duration, duration > 0.0)
}

/// Split a multi-input ffmpeg run's stderr into one banner section per input,
/// keyed by the input's file path (ffmpeg echoes it verbatim in each
/// `Input #N, … from '<path>':` header). Lets the batch thumbnail command pull
/// out each Clip's own Duration/dimensions to feed `finalize_thumb_banner`.
fn split_input_banners(stderr: &str) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    // Everything before the first "Input #" is the build/banner preamble — skip it.
    for section in stderr.split("Input #").skip(1) {
        let Some(s) = section.find("from '") else {
            continue;
        };
        let rest = &section[s + "from '".len()..];
        let Some(e) = rest.find("':") else {
            continue;
        };
        map.insert(rest[..e].to_string(), section.to_string());
    }
    map
}

/// Lazily render a poster-frame thumbnail for a Clip into the app cache dir.
/// Keyed by path + mtime so it regenerates if the file changes; returns the
/// cached JPG path (which the UI loads via `convertFileSrc`) plus a `healthy`
/// flag derived from the same ffmpeg run (see `ThumbResult`). The batch command
/// `clip_thumbnails` falls back to this for any Clip it couldn't render in bulk.
#[tauri::command]
pub(crate) async fn clip_thumbnail(app: AppHandle, path: String) -> Result<ThumbResult, String> {
    let key = cache_key(&path, "thumb")?;
    let out = cache_path(&app, "thumbs", &key, "jpg")?;
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

    // 1s in clears most intro fades, but a Clip shorter than that seek would yield
    // no keyframe past it — so a miss falls back to the first frame (`-ss 0`,
    // always a keyframe) rather than wrongly flagging a short-but-valid clip.
    let thumb_args = |seek: &str| -> Vec<String> {
        let mut a = vec!["-hide_banner".into()];
        a.extend(thumb_input_args(&path, seek));
        a.extend(thumb_output_args(&out_str, None));
        a
    };
    let mut output = run_ffmpeg(&app, thumb_args("1")).await?;
    if !output.status.success() || !out.exists() {
        // Sub-1s clip or a seek that landed past the end → retry from frame 0.
        output = run_ffmpeg(&app, thumb_args("0")).await?;
    }
    if !output.status.success() || !out.exists() {
        let _ = std::fs::remove_file(&out);
        return Err(format!(
            "thumbnail failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let (duration, _) =
        finalize_thumb_banner(&app, &path, &String::from_utf8_lossy(&output.stderr));
    Ok(ThumbResult {
        path: out_str,
        healthy: duration > 0.0,
        duration,
    })
}

/// One Clip's result from the batch thumbnail command.
#[derive(Serialize)]
pub(crate) struct BatchThumb {
    /// The *source* Clip path — the request key the frontend maps results back by.
    path: String,
    /// The cached poster JPG path on success (load via `convertFileSrc`), or
    /// `None` when even the per-Clip fallback couldn't render it (corrupt file).
    thumb: Option<String>,
    healthy: bool,
    duration: f64,
}

/// Batch-render poster thumbnails for several Clips in a *single* ffmpeg process
/// (N inputs → N mapped single-frame outputs), amortizing the ~75 ms per-process
/// spawn+init floor across the whole batch — roughly 2x faster than one spawn per
/// Clip once the keyframe-only decode (see `thumb_input_args`) made the thumbnail
/// overhead-bound again. Already-cached Clips resolve with no ffmpeg.
///
/// Robustness: ffmpeg opens *all* inputs before producing any output, so one
/// unreadable Clip aborts the entire batch (zero outputs). We therefore treat the
/// batch as best-effort — for every requested Clip whose output didn't appear
/// (a sub-1s seek miss, or a sibling poisoning the batch) we fall back to the
/// single `clip_thumbnail`, which retries `-ss 0` and reports corruption per file.
/// So a bad Clip costs its batch the bulk speedup but never blocks its neighbours.
#[tauri::command]
pub(crate) async fn clip_thumbnails(
    app: AppHandle,
    paths: Vec<String>,
) -> Result<Vec<BatchThumb>, String> {
    let mut results: Vec<BatchThumb> = Vec::with_capacity(paths.len());
    // (source path, output PathBuf, output path string) for the cache misses.
    let mut to_gen: Vec<(String, PathBuf, String)> = Vec::new();

    for path in paths {
        // A key/cache-dir failure is per-Clip — record it as unrenderable rather
        // than failing the whole batch.
        let Ok(key) = cache_key(&path, "thumb") else {
            results.push(BatchThumb {
                path,
                thumb: None,
                healthy: false,
                duration: 0.0,
            });
            continue;
        };
        let Ok(out) = cache_path(&app, "thumbs", &key, "jpg") else {
            results.push(BatchThumb {
                path,
                thumb: None,
                healthy: false,
                duration: 0.0,
            });
            continue;
        };
        let out_str = out.to_string_lossy().to_string();
        if out.exists() {
            // Cache hit — healthy by construction (see clip_thumbnail), duration
            // unknown without a banner (the frontend falls back to a probe).
            results.push(BatchThumb {
                path,
                thumb: Some(out_str),
                healthy: true,
                duration: 0.0,
            });
        } else {
            to_gen.push((path, out, out_str));
        }
    }

    if !to_gen.is_empty() {
        // One ffmpeg: every Clip's keyframe-only input, then every mapped poster
        // output (input index i → `-map i:v`), all seeking to 1s.
        let mut args: Vec<String> = vec!["-hide_banner".into()];
        for (path, _, _) in &to_gen {
            args.extend(thumb_input_args(path, "1"));
        }
        for (i, (_, _, out_str)) in to_gen.iter().enumerate() {
            args.extend(thumb_output_args(out_str, Some(&format!("{i}:v"))));
        }
        // A non-zero exit (e.g. an aborted batch) still yields per-input banners on
        // stderr for whatever opened; the per-output existence check below decides
        // success, so ignore the status and just capture stderr.
        let stderr = match run_ffmpeg(&app, args).await {
            Ok(o) => String::from_utf8_lossy(&o.stderr).into_owned(),
            Err(_) => String::new(),
        };
        let banners = split_input_banners(&stderr);

        for (path, out, out_str) in to_gen {
            if out.exists() {
                // Rendered in the batch — derive health/duration from its own
                // banner section (and warm the probe cache). A present output with
                // no parseable banner still decoded a frame, so treat it healthy.
                let (duration, healthy) = match banners.get(&path) {
                    Some(section) => finalize_thumb_banner(&app, &path, section),
                    None => (0.0, true),
                };
                results.push(BatchThumb {
                    path,
                    thumb: Some(out_str),
                    healthy,
                    duration,
                });
            } else {
                // Missing — fall back to the single command's full retry/health path.
                let _ = std::fs::remove_file(&out); // clear any partial write
                match clip_thumbnail(app.clone(), path.clone()).await {
                    Ok(r) => results.push(BatchThumb {
                        path,
                        thumb: Some(r.path),
                        healthy: r.healthy,
                        duration: r.duration,
                    }),
                    Err(_) => results.push(BatchThumb {
                        path,
                        thumb: None,
                        healthy: false,
                        duration: 0.0,
                    }),
                }
            }
        }
    }

    Ok(results)
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
    let cache = cache_path(&app, "waveforms", &key, "json")?;
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

    // "fs3" bumps the cache when the pixels changed (libav's swscale+jpeg-encoder
    // strip differs from the sidecar's mjpeg output), so old sprites regenerate.
    let key = cache_key(&path, &format!("fs3_{cols}"))?;
    let out = cache_path(&app, "filmstrips", &key, "jpg")?;
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

    // Preferred: in-process libav. It opens the Clip once and seek-decodes only
    // the `cols` keyframes the sprite needs, so its cost scales with `cols` (~24)
    // rather than the Clip's keyframe count (~600) — faster than even NVDEC on
    // short-GOP clips, and hardware-independent. Decode is blocking, so run it off
    // the async runtime. Any failure (or a Clip libav can't handle) falls through
    // to the sidecar GPU/CPU path below, so behaviour never regresses.
    if !LIBAV_DISABLED.load(Ordering::Relaxed) {
        let (p, o) = (path.clone(), out_str.clone());
        let (c, fw, dur) = (opts.cols, opts.frame_width, opts.duration);
        let res =
            tauri::async_runtime::spawn_blocking(move || filmstrip_libav(&p, c, fw, dur, &o)).await;
        match res {
            Ok(Ok(())) if out.exists() => return Ok(out_str),
            _ => {
                let _ = std::fs::remove_file(&out); // clear any partial libav output
            }
        }
    }

    // Fallback tier 1: NVIDIA GPU decode (NVDEC) via the sidecar. ShadowPlay
    // clips are short-GOP (~2 keyframes/s), so even the keyframe-only CPU path
    // decodes hundreds of 1440p frames — ~8 s on a 5-min clip. NVDEC cuts that
    // ~6x. On any GPU failure we fall back to the CPU args, and a clear "no
    // hardware decode here" signal disables the GPU attempt for the rest of the
    // session so we don't pay a doomed spawn per filmstrip.
    if !NVDEC_DISABLED.load(Ordering::Relaxed) {
        match run_ffmpeg(&app, filmstrip_args(&path, &opts, &out_str, true)).await {
            Ok(o) if o.status.success() && out.exists() => return Ok(out_str),
            Ok(o) => {
                let _ = std::fs::remove_file(&out); // clear any partial GPU output
                if cuda_unavailable(&String::from_utf8_lossy(&o.stderr)) {
                    NVDEC_DISABLED.store(true, Ordering::Relaxed);
                }
            }
            Err(_) => {
                let _ = std::fs::remove_file(&out);
            }
        }
    }

    // CPU fallback (also the steady-state path once NVDEC is known-unavailable).
    run_ffmpeg_checked(
        &app,
        filmstrip_args(&path, &opts, &out_str, false),
        "filmstrip",
        Some(&out),
    )
    .await?;
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
/// path corresponds to `path` (newest wins if the same path was trashed more
/// than once), and restore just that one.
///
/// Windows' Recycle Bin reports the original path with its final extension
/// dropped (`a.mp4` → `a`), and `restore_all` puts the file back at *that*
/// stripped path — so after restoring we rename it to the real target (with its
/// extension) the caller asked for. Without this the restored clip would lose
/// its `.mp4` and the library scan would no longer see it as a video (#43).
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
    let idx = pick_restore_index(&keyed, &target, restore_paths_match)
        .ok_or("Couldn't find that clip in the Recycle Bin.")?;
    let newest = items.into_iter().nth(idx).unwrap();
    let landed_at = newest.original_path(); // where restore_all will put it
    restore_all([newest]).map_err(|e| e.to_string())?;
    // If the trash crate restored to the extension-stripped path, move it to the
    // path the caller actually holds. Guarded so a correct restore is left alone.
    if landed_at != target && landed_at.exists() && !target.exists() {
        std::fs::rename(&landed_at, &target).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Pure selection step for [`restore_clip`]: from `(original_path, time_deleted)`
/// trash entries, return the index of the entry matching `target` (per the given
/// path matcher) that was deleted most recently — the copy the user just trashed
/// when the same path has been deleted more than once. Returns `None` when
/// nothing matches. The matcher is a parameter so both OS semantics stay
/// unit-testable on every platform.
fn pick_restore_index(
    entries: &[(PathBuf, i64)],
    target: &Path,
    matches: impl Fn(&Path, &Path) -> bool,
) -> Option<usize> {
    entries
        .iter()
        .enumerate()
        .filter(|(_, (p, _))| matches(p, target))
        .max_by_key(|(_, (_, t))| *t)
        .map(|(i, _)| i)
}

/// Whether a trashed entry's reported original path `entry` refers to the same
/// Clip as `target` (the native path the app holds). The comparison rules differ
/// per OS trash implementation, so this just dispatches to the platform's pure
/// matcher; both matchers compile everywhere so both are tested everywhere.
fn restore_paths_match(entry: &Path, target: &Path) -> bool {
    #[cfg(windows)]
    let matched = restore_paths_match_windows(entry, target);
    #[cfg(not(windows))]
    let matched = restore_paths_match_unix(entry, target);
    matched
}

/// Windows Recycle Bin semantics: the bin lists the original path with its
/// **final extension dropped** (`a.mp4` → `a`), which broke a naive `==` match
/// (#43), so we accept the target both with and without its extension.
/// Comparison is case- and separator-insensitive because Windows paths are
/// case-folded and the two sources can differ on `/` vs `\`.
#[cfg_attr(not(windows), allow(dead_code))]
fn restore_paths_match_windows(entry: &Path, target: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().replace('/', "\\").to_lowercase();
    let e = norm(entry);
    e == norm(target) || e == norm(&target.with_extension(""))
}

/// Linux (freedesktop) trash semantics: the trash keeps the full original path
/// — extension included — and Unix paths are case-sensitive with `/` separators,
/// so this is an exact compare.
#[cfg_attr(windows, allow(dead_code))]
fn restore_paths_match_unix(entry: &Path, target: &Path) -> bool {
    entry == target
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

/// Copy the Clip file itself onto the clipboard (Windows CF_HDROP file list) so
/// it can be pasted straight into Explorer, Discord, email, etc. — making clips
/// easy to send without first locating them on disk.
#[cfg(windows)]
#[tauri::command]
pub(crate) fn copy_clip(path: String) -> Result<(), String> {
    use clipboard_win::{formats, Clipboard, Setter};
    // Hold the clipboard open for the write; retry a few times since another
    // process may briefly own it.
    let _clip = Clipboard::new_attempts(10).map_err(|e| format!("clipboard open: {e}"))?;
    formats::FileList
        .write_clipboard(&[path])
        .map_err(|e| format!("clipboard write: {e}"))
}

/// Linux: put the Clip on the clipboard as `text/uri-list` (one percent-encoded
/// `file://` URL) so it pastes as a file into file managers and chat apps. There
/// is no in-process clipboard here, so this shells out to the session's
/// clipboard tool: `wl-copy` (wl-clipboard) on Wayland, falling back to `xclip`
/// on X11. Only `text/uri-list` is offered — both tools own the selection with a
/// single MIME type per invocation, so also offering GNOME's
/// `x-special/gnome-copied-files` would clobber the uri-list write; GNOME Files
/// accepts plain `text/uri-list` pastes anyway.
#[cfg(target_os = "linux")]
#[tauri::command]
pub(crate) fn copy_clip(path: String) -> Result<(), String> {
    // text/uri-list lines are CRLF-terminated (RFC 2483).
    let payload = format!("{}\r\n", file_uri(&path)).into_bytes();
    // Prefer wl-copy only when a Wayland session is actually up: on plain X11
    // an installed wl-copy exists but can't connect, and xclip under XWayland
    // still reaches the shared clipboard.
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        if let Some(res) = clipboard_write("wl-copy", &["--type", "text/uri-list"], &payload) {
            return res;
        }
    }
    if let Some(res) = clipboard_write(
        "xclip",
        &["-selection", "clipboard", "-t", "text/uri-list"],
        &payload,
    ) {
        return res;
    }
    Err(
        "No clipboard tool found: install wl-clipboard (wl-copy) on Wayland or xclip on X11."
            .into(),
    )
}

/// Pipe `payload` into a clipboard tool. Returns `None` when the tool isn't on
/// the PATH (so the caller can try the next one), `Some(result)` once a tool was
/// found and run.
#[cfg(target_os = "linux")]
fn clipboard_write(tool: &str, args: &[&str], payload: &[u8]) -> Option<Result<(), String>> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = match Command::new(tool)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => return Some(Err(format!("{tool}: {e}"))),
    };
    if let Some(mut stdin) = child.stdin.take() {
        if let Err(e) = stdin.write_all(payload) {
            let _ = child.kill();
            let _ = child.wait();
            return Some(Err(format!("{tool}: {e}")));
        }
        // Drop stdin so the tool sees EOF and takes the selection.
    }
    match child.wait() {
        Ok(status) if status.success() => Some(Ok(())),
        Ok(status) => Some(Err(format!("{tool} exited with {status}"))),
        Err(e) => Some(Err(format!("{tool}: {e}"))),
    }
}

/// Everything else (neither Windows nor Linux): no clipboard backend wired up.
#[cfg(not(any(windows, target_os = "linux")))]
#[tauri::command]
pub(crate) fn copy_clip(_path: String) -> Result<(), String> {
    Err("copying clips to the clipboard is not supported on this platform".into())
}

/// Percent-encode an absolute Unix path into a `file://` URL for
/// `text/uri-list`. Keeps RFC 3986 unreserved characters and `/` literal and
/// encodes every other byte of the UTF-8 string, so spaces, `#`, `?`, `%`, and
/// non-ASCII names round-trip. Pure so it unit-tests on every platform.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn file_uri(path: &str) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(path.len() + 7);
    out.push_str("file://");
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(b as char)
            }
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pb(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn split_input_banners_keys_each_section_by_its_path() {
        // A two-input batch run's stderr: each `Input #N … from '<path>':` header
        // delimits one Clip's banner. The output mjpeg streams trail the last
        // input section but carry no `from '…'`, so they don't add a key.
        let stderr = "\
ffmpeg version ...
Input #0, mov,mp4, from 'C:/clips/a.mp4':
  Duration: 00:00:15.10, start: 0.000000, bitrate: 8000 kb/s
  Stream #0:0: Video: h264, 1920x1080, 30 fps
Input #1, mov,mp4, from 'C:/clips/b.mp4':
  Duration: 00:05:00.11, start: 0.000000, bitrate: 28000 kb/s
  Stream #1:0: Video: h264, 2560x1440, 59.90 fps
  Stream #0:0: Video: mjpeg, 480x270, 60 fps
";
        let banners = split_input_banners(stderr);
        assert_eq!(banners.len(), 2);
        // Each section parses to its own Clip's metadata, not a neighbour's.
        let (d_a, w_a, h_a, _) = parse_ffmpeg_probe(&banners["C:/clips/a.mp4"]);
        assert!((d_a - 15.10).abs() < 0.01, "a duration {d_a}");
        assert_eq!((w_a, h_a), (1920, 1080));
        // b's section takes its *own* (first) Video line — the 1440p input, not
        // the trailing 480x270 output stream.
        let (d_b, w_b, h_b, fps_b) = parse_ffmpeg_probe(&banners["C:/clips/b.mp4"]);
        assert!((d_b - 300.11).abs() < 0.01, "b duration {d_b}");
        assert_eq!((w_b, h_b), (2560, 1440));
        assert!((fps_b - 59.90).abs() < 0.01, "b fps {fps_b}");
    }

    #[test]
    fn split_input_banners_handles_empty_and_preamble_only() {
        assert!(split_input_banners("").is_empty());
        assert!(split_input_banners("ffmpeg version 7 ... no inputs opened").is_empty());
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
        let idx =
            pick_restore_index(&entries, Path::new("C:/clips/a.mp4"), restore_paths_match).unwrap();
        assert_eq!(
            idx, 2,
            "should pick the most recently deleted matching entry"
        );
    }

    #[test]
    fn pick_restore_index_matches_only_the_target_path() {
        let entries = vec![(pb("C:/clips/a.mp4"), 100), (pb("C:/clips/b.mp4"), 200)];
        let idx =
            pick_restore_index(&entries, Path::new("C:/clips/a.mp4"), restore_paths_match).unwrap();
        assert_eq!(idx, 0);
    }

    #[test]
    fn pick_restore_index_none_when_no_match() {
        let entries = vec![(pb("C:/clips/a.mp4"), 100)];
        assert!(pick_restore_index(
            &entries,
            Path::new("C:/clips/missing.mp4"),
            restore_paths_match
        )
        .is_none());
        assert!(
            pick_restore_index(&[], Path::new("C:/clips/a.mp4"), restore_paths_match).is_none()
        );
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

    #[test]
    fn restore_paths_match_windows_tolerates_dropped_extension_and_case() {
        // The real bug (#43): the Recycle Bin lists the original path with its
        // final extension stripped, so the entry never == the path we pass.
        // Exercises the Windows matcher directly so it runs on every platform.
        let target = pb("C:/clips/Clip 2026.06.17.DVR.mp4");
        assert!(restore_paths_match_windows(
            Path::new("C:/clips/Clip 2026.06.17.DVR"),
            &target
        ));
        // Exact path (a future trash-crate fix that keeps the extension) still matches.
        assert!(restore_paths_match_windows(
            Path::new("C:/clips/Clip 2026.06.17.DVR.mp4"),
            &target
        ));
        // Case- and separator-insensitive (Windows paths are case-folded).
        assert!(restore_paths_match_windows(
            Path::new(r"c:\CLIPS\clip 2026.06.17.dvr"),
            &target
        ));
        // A genuinely different clip must not match.
        assert!(!restore_paths_match_windows(
            Path::new("C:/clips/Other Clip"),
            &target
        ));
    }

    #[test]
    fn restore_paths_match_unix_is_exact_and_case_sensitive() {
        // Linux (freedesktop) trash keeps the extension, so only the exact
        // original path matches.
        let target = pb("/home/y/Videos/Clip 2026.06.17.DVR.mp4");
        assert!(restore_paths_match_unix(
            Path::new("/home/y/Videos/Clip 2026.06.17.DVR.mp4"),
            &target
        ));
        // An extension-stripped entry must NOT match (that's a Windows quirk;
        // matching it here would restore the wrong file).
        assert!(!restore_paths_match_unix(
            Path::new("/home/y/Videos/Clip 2026.06.17.DVR"),
            &target
        ));
        // Unix paths are case-sensitive: a case-differing path is another file.
        assert!(!restore_paths_match_unix(
            Path::new("/home/y/videos/clip 2026.06.17.dvr.mp4"),
            &target
        ));
    }

    #[test]
    fn pick_restore_index_matches_extension_stripped_entries() {
        // Entries as the Recycle Bin actually reports them (no extension); target
        // is the full native path the app holds. Newest of the matches wins.
        // Uses the Windows matcher explicitly so the test runs everywhere.
        let entries = vec![
            (pb(r"C:\clips\a"), 100),
            (pb(r"C:\clips\b"), 150),
            (pb(r"C:\clips\a"), 200),
        ];
        let idx = pick_restore_index(
            &entries,
            Path::new(r"C:\clips\a.mp4"),
            restore_paths_match_windows,
        )
        .unwrap();
        assert_eq!(idx, 2);
    }

    #[test]
    fn file_uri_percent_encodes_reserved_and_non_ascii_bytes() {
        // Plain absolute path: only the scheme is added.
        assert_eq!(
            file_uri("/home/y/Videos/clip.mp4"),
            "file:///home/y/Videos/clip.mp4"
        );
        // Spaces and URL-special characters are percent-encoded so the URI
        // survives text/uri-list parsing in file managers.
        assert_eq!(
            file_uri("/home/y/My Clip #1? 50%.mp4"),
            "file:///home/y/My%20Clip%20%231%3F%2050%25.mp4"
        );
        // Non-ASCII names encode per UTF-8 byte.
        assert_eq!(file_uri("/home/y/é.mp4"), "file:///home/y/%C3%A9.mp4");
    }
}
