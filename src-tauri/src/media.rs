//! Pure media-encoding building blocks: the FFmpeg argument vectors for GIF /
//! WebP / filmstrip / waveform exports, plus the bitrate / scale / NVENC math
//! the compress path leans on. Everything here is pure (no process spawning) so
//! it can be unit-tested without ffmpeg.

use std::sync::atomic::AtomicBool;

/// Options for a GIF / animated-WebP export.
pub(crate) struct GifOpts {
    pub(crate) fps: u32,
    pub(crate) width: u32,
    /// true → animated WebP (true-colour), false → GIF (palette).
    pub(crate) webp: bool,
}

/// Build the FFmpeg args to render the Region `[start, start+dur]` of `input`
/// to a looping GIF or animated WebP at `output`. GIF uses a single-pass
/// `split → palettegen → paletteuse` graph so each clip gets an optimal palette
/// (far better than the default 256-colour quantizer); WebP is a true-colour
/// single pass via libwebp. `fps`/`width` are clamped to sane bounds; the scale
/// keeps aspect ratio (`-1`). Pure — collision-safe naming happens in the caller.
pub(crate) fn gif_args(
    input: &str,
    start: f64,
    dur: f64,
    speed: f64,
    opts: &GifOpts,
    output: &str,
) -> Vec<String> {
    let fps = opts.fps.clamp(1, 50);
    let width = opts.width.clamp(64, 1920);
    let scale = format!("scale={width}:-1:flags=lanczos");
    // Retime first (when not 1x) so `fps` resamples the already-stretched
    // timeline. The input-side `-t {dur}` below still reads the same source
    // region, so the output simply plays it over `dur / speed` seconds.
    let setpts = speed_setpts_filter(speed)
        .map(|s| format!("{s},"))
        .unwrap_or_default();
    // -ss before -i: fast keyframe seek, consistent with Trim/Compress.
    let mut a: Vec<String> = vec![
        "-ss".into(),
        format!("{start}"),
        "-t".into(),
        format!("{dur}"),
        "-i".into(),
        input.to_string(),
    ];
    if opts.webp {
        a.extend([
            "-vf".into(),
            format!("{setpts}fps={fps},{scale}"),
            "-c:v".into(),
            "libwebp".into(),
            "-lossless".into(),
            "0".into(),
            "-q:v".into(),
            "75".into(),
            "-loop".into(),
            "0".into(),
            "-an".into(),
            "-y".into(),
            output.to_string(),
        ]);
    } else {
        let filter = format!(
            "{setpts}fps={fps},{scale},split[s0][s1];[s0]palettegen=stats_mode=diff[p];\
             [s1][p]paletteuse=dither=bayer:bayer_scale=5:diff_mode=rectangle"
        );
        a.extend([
            "-filter_complex".into(),
            filter,
            "-loop".into(),
            "0".into(),
            "-an".into(),
            "-y".into(),
            output.to_string(),
        ]);
    }
    a
}

/// The shared FFmpeg input/seek/duration/stream-map prefix for the Region
/// `[start, start+dur]` of `input`, common to Trim and Compress. `-ss` before
/// `-i` is the fast keyframe seek used everywhere. `video_map` selects the video
/// stream(s) — `"0:v?"` keeps every video stream (a stream-copy Trim), `"0:v:0"`
/// picks the first (a single-stream re-encode). Audio is mapped (`0:a?`) only
/// when `include_audio`; simply omitting the map is how audio is dropped
/// losslessly (no `-an` needed alongside `-c copy`). Pure — the per-encoder tail
/// and output path are appended by the caller.
pub(crate) fn input_segment(
    path: &str,
    start: f64,
    dur: f64,
    video_map: &str,
    include_audio: bool,
) -> Vec<String> {
    let mut a = vec![
        "-ss".into(),
        format!("{start}"),
        "-i".into(),
        path.to_string(),
        "-t".into(),
        format!("{dur}"),
        "-map".into(),
        video_map.to_string(),
    ];
    if include_audio {
        a.push("-map".into());
        a.push("0:a?".into());
    }
    a
}

/// Build FFmpeg args to export just the Region's audio (video dropped via
/// `-vn`). `mp3` selects MP3 (libmp3lame, always a re-encode); otherwise M4A,
/// where `copy` chooses a lossless `-c:a copy` (valid because ShadowPlay records
/// AAC) and `copy == false` falls back to an AAC re-encode for non-AAC sources.
/// `+faststart` keeps the m4a web/Discord-friendly. Pure.
pub(crate) fn audio_args(
    input: &str,
    start: f64,
    dur: f64,
    speed: f64,
    mp3: bool,
    copy: bool,
    output: &str,
) -> Vec<String> {
    let mut a = vec![
        "-ss".into(),
        format!("{start}"),
        "-t".into(),
        format!("{dur}"),
        "-i".into(),
        input.to_string(),
        "-vn".into(),
        "-map".into(),
        "0:a:0".into(),
    ];
    // A speed change time-stretches the audio (pitch preserved). This forces a
    // re-encode — `-c:a copy` can't retime — so the copy branch is disabled
    // whenever a tempo filter is present. The input-side `-t {dur}` reads the
    // same source region; atempo plays it over `dur / speed` seconds.
    let atempo = atempo_chain(speed);
    if let Some(af) = &atempo {
        a.push("-filter:a".into());
        a.push(af.clone());
    }
    if mp3 {
        a.extend([
            "-c:a".into(),
            "libmp3lame".into(),
            "-q:a".into(),
            "2".into(),
        ]);
    } else if copy && atempo.is_none() {
        a.extend([
            "-c:a".into(),
            "copy".into(),
            "-movflags".into(),
            "+faststart".into(),
        ]);
    } else {
        a.extend([
            "-c:a".into(),
            "aac".into(),
            "-b:a".into(),
            "192k".into(),
            "-movflags".into(),
            "+faststart".into(),
        ]);
    }
    a.extend(["-y".into(), output.to_string()]);
    a
}

/// Options for waveform extraction.
pub(crate) struct WaveformOpts {
    /// Mono decode rate — low (a few kHz) keeps the PCM small; the peaks
    /// reduction makes the exact rate visually irrelevant.
    pub(crate) sample_rate: u32,
}

/// Build the FFmpeg args to decode `input`'s audio to raw mono s16le PCM on
/// stdout (`pipe:1`), with video dropped. The caller reads the bytes and reduces
/// them with `peaks`. Pure.
pub(crate) fn waveform_args(input: &str, opts: &WaveformOpts) -> Vec<String> {
    vec![
        "-i".into(),
        input.to_string(),
        "-vn".into(),
        "-ac".into(),
        "1".into(),
        "-ar".into(),
        opts.sample_rate.to_string(),
        "-f".into(),
        "s16le".into(),
        "pipe:1".into(),
    ]
}

/// Reduce signed-16 PCM samples to `buckets` normalised amplitude values in
/// `[0, 1]`, time-ordered, for drawing a waveform along the Timeline. Each
/// bucket holds the RMS (average energy) of its slice — not the peak — so a clip
/// that's loud but dynamic still shows contrast instead of every bucket pegging
/// to the height from transient spikes. The set is then normalised to the
/// loudest bucket so the waveform fills the height relative to the Clip's own
/// average. Silence (or no audio → no samples) yields all zeros. Pure.
pub(crate) fn peaks(samples: &[i16], buckets: usize) -> Vec<f32> {
    let mut out = vec![0f32; buckets];
    if buckets == 0 || samples.is_empty() {
        return out;
    }
    let n = samples.len();
    for (i, bucket) in out.iter_mut().enumerate() {
        let start = (i * n / buckets).min(n);
        let end = (((i + 1) * n / buckets).max(start + 1)).min(n);
        if start >= end {
            continue; // more buckets than samples → leave this one at 0
        }
        let slice = &samples[start..end];
        let sum_sq: f64 = slice.iter().map(|&s| (s as f64) * (s as f64)).sum();
        *bucket = (sum_sq / slice.len() as f64).sqrt() as f32;
    }
    let max = out.iter().copied().fold(0f32, f32::max);
    if max > 0.0 {
        for b in out.iter_mut() {
            *b /= max;
        }
    }
    out
}

/// Options for a filmstrip sprite.
pub(crate) struct FilmstripOpts {
    /// Number of evenly-spaced frames to sample (tiled into one row).
    pub(crate) cols: u32,
    /// Width of each frame cell in px (height keeps aspect).
    pub(crate) frame_width: u32,
    /// Clip duration in seconds, used to space the samples across the whole Clip.
    pub(crate) duration: f64,
}

/// Build the FFmpeg args to render a horizontal filmstrip sprite of `input`:
/// `cols` frames sampled evenly across the Clip, scaled to `frame_width`, tiled
/// into a single 1-row image at `output`. Pure.
///
/// One input, decoder set to `-skip_frame nokey` so it emits *only keyframes* —
/// then `fps=cols/duration` resamples that sparse keyframe stream to exactly
/// `cols` frames evenly spaced across the whole Clip, which `tile` stitches into
/// one row. `-frames:v 1` stops after the single tiled output.
///
/// Why this shape (it was measured against the alternatives on a 1080p60 clip):
///   * `fps=cols/duration` WITHOUT `-skip_frame` is the original slow path — it
///     decodes *every* frame just to keep `cols` (~90 s on a 5-min Clip). The
///     `-skip_frame nokey` is what makes it cheap: only keyframes are decoded.
///   * One input-side `-ss`/`-i` per cell (the previous approach) re-opens and
///     re-inits the demuxer once *per cell*; that per-open overhead dominates and
///     scales linearly with `cols` (~5x slower at 24 cols here). A single open
///     plus keyframe-only decode wins and stays ~flat as `cols` grows.
///
/// Frames snap to the keyframe at/just before each sample point rather than the
/// exact timestamp — invisible for a decorative scrub strip, and the frontend's
/// hover preview still shows the precise hovered time as text. Even spacing is
/// preserved (the frontend maps hover x → cell assuming even spacing).
///
/// Without a known duration we can't set the resample rate, so fall back to
/// sampling one keyframe per second from the head (still keyframe-only).
pub(crate) fn filmstrip_args(input: &str, opts: &FilmstripOpts, output: &str) -> Vec<String> {
    let cols = opts.cols.max(1);
    let w = opts.frame_width;

    // `-skip_frame nokey` is a decoder option: place it before `-i` so it applies
    // to the input's decoder, making it emit only keyframes (the speed win).
    let vf = if opts.duration > 0.0 {
        // cols frames spread across the whole Clip.
        let fps = cols as f64 / opts.duration;
        format!("fps={fps:.6},scale={w}:-2:flags=lanczos,tile={cols}x1")
    } else {
        // Unknown duration: can't place the samples, so take a keyframe a second
        // from the head — same fallback as before, just keyframe-only now.
        format!("fps=1.000000,scale={w}:-2:flags=lanczos,tile={cols}x1")
    };

    vec![
        // One decode thread: keyframe-only sampling is light and several of
        // these run concurrently for card-hover sprites — cap the per-process
        // footprint (see clip_thumbnail). Decoder option, so it precedes -i.
        "-threads".into(),
        "1".into(),
        "-skip_frame".into(),
        "nokey".into(),
        "-i".into(),
        input.to_string(),
        "-frames:v".into(),
        "1".into(),
        "-vf".into(),
        vf,
        "-an".into(),
        "-q:v".into(),
        "4".into(),
        "-y".into(),
        output.to_string(),
    ]
}

/// Compute the video bitrate ladder (kbps) for a size-targeted encode.
/// Returns (video_kbps, maxrate_kbps, bufsize_kbps). `dur` is the Region length
/// in seconds and must be > 0 (callers guarantee this via the end > start guard).
pub(crate) fn size_target_bitrate(target_mb: f64, dur: f64, audio_kbps: f64) -> (f64, f64, f64) {
    let target = target_mb.max(1.0);
    // Aim under target to leave headroom for container overhead + VBR variance.
    let total_kbps = (target * 1024.0 * 8.0 / dur) * 0.90;
    let v_kbps = (total_kbps - audio_kbps).max(300.0);
    // Cap the peak close to the average so two-pass can't blow the budget.
    (v_kbps, v_kbps * 1.10, v_kbps * 1.5)
}

/// The ffmpeg scale filter for a quality-mode resolution preset (e.g. "720"),
/// or None to keep the source resolution ("source" / unrecognised). Downscales
/// to the target height but never upscales (`min(ih, H)`); width auto-keeps the
/// aspect ratio and stays even (`-2`). Pure.
pub(crate) fn quality_scale_filter(token: &str) -> Option<String> {
    let h = match token {
        "480" => 480,
        "720" => 720,
        "1080" => 1080,
        _ => return None,
    };
    Some(format!("scale=-2:'min(ih,{h})'"))
}

/// FFmpeg `crop` filter for a source-pixel rectangle `(x, y, w, h)`, or None
/// when there's no crop. Note ffmpeg's argument order is `crop=out_w:out_h:x:y`
/// (size first, then top-left offset) — not `x:y:w:h`. Pure.
pub(crate) fn crop_filter(rect: Option<(u32, u32, u32, u32)>) -> Option<String> {
    rect.map(|(x, y, w, h)| format!("crop={w}:{h}:{x}:{y}"))
}

/// Compose the optional crop + scale filters into a single `-vf` value (ffmpeg
/// allows only one `-vf`). Crop must come first so the source frame is cropped
/// and only then downscaled — reversing them would crop the already-scaled frame
/// and keep the wrong region. None when neither filter is present. Pure.
pub(crate) fn compose_vf(crop: Option<String>, scale: Option<String>) -> Option<String> {
    match (crop, scale) {
        (Some(c), Some(s)) => Some(format!("{c},{s}")),
        (Some(c), None) => Some(c),
        (None, Some(s)) => Some(s),
        (None, None) => None,
    }
}

/// True when `speed` is effectively 1x (no retime needed). The speed helpers all
/// short-circuit on this so the 1x path stays byte-identical to the old args.
fn is_unit_speed(speed: f64) -> bool {
    (speed - 1.0).abs() < 1e-6
}

/// FFmpeg video `setpts` filter that retimes a clip to play at `speed`× (2.0 →
/// half the timestamps → twice as fast; 0.5 → double the timestamps → half
/// speed). None at 1x. Pure — composes into the single `-vf` via
/// [`compose_vf_speed`].
pub(crate) fn speed_setpts_filter(speed: f64) -> Option<String> {
    if is_unit_speed(speed) {
        return None;
    }
    Some(format!("setpts={:.6}*PTS", 1.0 / speed))
}

/// FFmpeg audio `atempo` chain that changes tempo to `speed`× while preserving
/// pitch. A single `atempo` only accepts a 0.5..=2.0 factor, so out-of-range
/// speeds are decomposed into a chain of 2.0 / 0.5 stages plus a remainder
/// (4x → `atempo=2.0,atempo=2.0`; 0.25x → `atempo=0.5,atempo=0.5`). None at 1x.
/// Pure. Note: this forces an audio re-encode (incompatible with `-c:a copy`).
pub(crate) fn atempo_chain(speed: f64) -> Option<String> {
    if is_unit_speed(speed) {
        return None;
    }
    let mut remaining = speed;
    let mut stages: Vec<String> = Vec::new();
    while remaining > 2.0 + 1e-9 {
        stages.push("atempo=2.0".into());
        remaining /= 2.0;
    }
    while remaining < 0.5 - 1e-9 {
        stages.push("atempo=0.5".into());
        remaining *= 2.0;
    }
    stages.push(format!("atempo={remaining:.6}"));
    Some(stages.join(","))
}

/// Like [`input_segment`] but for a speed-changed re-encode: the `-t` output cap
/// is set to `out_dur` (the *post-retime* length, `dur / speed`) instead of the
/// source-region length `dur`. `-ss`/`-i` still cover the source region
/// `[start, start+dur]`; only the output-duration cap changes, because `setpts`
/// stretches/compresses the output and a `-t {dur}` would truncate slow-motion.
/// Pure.
pub(crate) fn input_segment_out(
    path: &str,
    start: f64,
    out_dur: f64,
    video_map: &str,
    include_audio: bool,
) -> Vec<String> {
    let mut a = vec![
        "-ss".into(),
        format!("{start}"),
        "-i".into(),
        path.to_string(),
        "-t".into(),
        format!("{out_dur}"),
        "-map".into(),
        video_map.to_string(),
    ];
    if include_audio {
        a.push("-map".into());
        a.push("0:a?".into());
    }
    a
}

/// Compose crop + scale + speed (`setpts`) into the single allowed `-vf`. Crop
/// and scale keep their order from [`compose_vf`] (crop before scale); `setpts`
/// is appended LAST — it only rewrites timestamps, so it's order-independent
/// relative to the pixel filters and stays out of their way. None when all three
/// are absent. Pure.
pub(crate) fn compose_vf_speed(
    crop: Option<String>,
    scale: Option<String>,
    setpts: Option<String>,
) -> Option<String> {
    let pixel = compose_vf(crop, scale);
    match (pixel, setpts) {
        (Some(p), Some(s)) => Some(format!("{p},{s}")),
        (Some(p), None) => Some(p),
        (None, Some(s)) => Some(s),
        (None, None) => None,
    }
}

/// Set once NVENC has proven unsupported on this machine, so later compresses
/// skip the doomed GPU attempt. Process-global; resets on app restart.
pub(crate) static NVENC_DISABLED: AtomicBool = AtomicBool::new(false);

/// True when ffmpeg's stderr indicates NVENC is unsupported on this system
/// (no NVIDIA GPU / driver / encoder), as opposed to a transient or
/// clip-specific encode error. Conservative on purpose: an unrecognized
/// failure is NOT treated as "unavailable", so a one-off hiccup never
/// permanently disables the GPU path.
pub(crate) fn nvenc_unavailable(stderr: &str) -> bool {
    const MARKERS: [&str; 6] = [
        "Cannot load nvcuda",
        "Cannot load libcuda",
        "No NVENC capable devices found",
        "Cannot init CUDA",
        "Unknown encoder 'h264_nvenc'",
        "Cannot open encoder",
    ];
    MARKERS.iter().any(|m| stderr.contains(m))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Find the value passed to a single-occurrence flag in an arg vector.
    fn flag_val<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .map(|s| s.as_str())
    }

    #[test]
    fn nvenc_unavailable_detects_unsupported_but_not_transient() {
        // Signatures that mean "no usable NVENC on this machine".
        assert!(nvenc_unavailable(
            "[h264_nvenc @ ...] Cannot load nvcuda.dll"
        ));
        assert!(nvenc_unavailable("No NVENC capable devices found"));
        assert!(nvenc_unavailable("Unknown encoder 'h264_nvenc'"));
        // A generic / transient failure must NOT disable the GPU path.
        assert!(!nvenc_unavailable(
            "Error while opening encoder - maybe incorrect parameters"
        ));
        assert!(!nvenc_unavailable("Conversion failed!"));
        assert!(!nvenc_unavailable(""));
    }

    #[test]
    fn quality_scale_filter_downscales_without_upscaling() {
        assert_eq!(
            quality_scale_filter("720"),
            Some("scale=-2:'min(ih,720)'".to_string())
        );
        assert_eq!(
            quality_scale_filter("1080"),
            Some("scale=-2:'min(ih,1080)'".to_string())
        );
        assert_eq!(
            quality_scale_filter("480"),
            Some("scale=-2:'min(ih,480)'".to_string())
        );
        // "source" and legacy low/medium/high values keep the native resolution.
        assert_eq!(quality_scale_filter("source"), None);
        assert_eq!(quality_scale_filter("medium"), None);
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
    fn gif_args_builds_palette_graph_with_region_and_defaults() {
        let opts = GifOpts {
            fps: 15,
            width: 640,
            webp: false,
        };
        let a = gif_args("in.mp4", 2.5, 4.0, 1.0, &opts, "out.gif");
        // Region seek + duration.
        assert_eq!(flag_val(&a, "-ss"), Some("2.5"));
        assert_eq!(flag_val(&a, "-t"), Some("4"));
        assert_eq!(flag_val(&a, "-i"), Some("in.mp4"));
        // Single-pass palette graph for quality.
        let filter = flag_val(&a, "-filter_complex").unwrap();
        assert!(filter.contains("fps=15"), "filter: {filter}");
        assert!(
            filter.contains("scale=640:-1:flags=lanczos"),
            "filter: {filter}"
        );
        assert!(filter.contains("palettegen"), "filter: {filter}");
        assert!(filter.contains("paletteuse"), "filter: {filter}");
        // Loops forever, no audio, writes the gif.
        assert_eq!(flag_val(&a, "-loop"), Some("0"));
        assert!(a.contains(&"-an".to_string()));
        assert_eq!(a.last().unwrap(), "out.gif");
    }

    #[test]
    fn gif_args_webp_is_truecolour_single_pass() {
        let opts = GifOpts {
            fps: 24,
            width: 800,
            webp: true,
        };
        let a = gif_args("in.mp4", 0.0, 3.0, 1.0, &opts, "out.webp");
        // WebP uses a plain -vf scale chain via libwebp, no palette.
        let vf = flag_val(&a, "-vf").unwrap();
        assert!(vf.contains("fps=24"), "vf: {vf}");
        assert!(vf.contains("scale=800:-1:flags=lanczos"), "vf: {vf}");
        assert!(!a.iter().any(|s| s.contains("palettegen")));
        assert_eq!(flag_val(&a, "-c:v"), Some("libwebp"));
        assert_eq!(flag_val(&a, "-loop"), Some("0"));
        assert_eq!(a.last().unwrap(), "out.webp");
    }

    #[test]
    fn gif_args_clamps_fps_and_width() {
        // fps 0 → floor 1; absurd width → ceiling 1920.
        let opts = GifOpts {
            fps: 0,
            width: 9999,
            webp: false,
        };
        let a = gif_args("in.mp4", 0.0, 1.0, 1.0, &opts, "out.gif");
        let filter = flag_val(&a, "-filter_complex").unwrap();
        assert!(filter.contains("fps=1"), "filter: {filter}");
        assert!(filter.contains("scale=1920:-1"), "filter: {filter}");
    }

    #[test]
    fn filmstrip_args_resamples_keyframes_evenly() {
        let opts = FilmstripOpts {
            cols: 10,
            frame_width: 160,
            duration: 20.0,
        };
        let a = filmstrip_args("in.mp4", &opts, "out.jpg");
        // One input open, decoder restricted to keyframes (the speed win).
        assert_eq!(a.iter().filter(|s| *s == "-i").count(), 1);
        assert_eq!(flag_val(&a, "-i"), Some("in.mp4"));
        assert_eq!(flag_val(&a, "-skip_frame"), Some("nokey"));
        assert_eq!(flag_val(&a, "-threads"), Some("1"));
        assert_eq!(flag_val(&a, "-frames:v"), Some("1"));
        // No per-cell seeks any more.
        assert_eq!(a.iter().filter(|s| *s == "-ss").count(), 0);
        // Resample to cols frames over the whole clip (10/20 = 0.5 fps), then tile.
        let f = flag_val(&a, "-vf").unwrap();
        assert!(f.contains("fps=0.500000"), "vf: {f}");
        assert!(f.contains("scale=160:-2:flags=lanczos"), "vf: {f}");
        assert!(f.contains("tile=10x1"), "vf: {f}");
        assert_eq!(a.last().unwrap(), "out.jpg");
    }

    #[test]
    fn filmstrip_args_falls_back_when_duration_unknown() {
        // duration 0 can't set the resample rate → sample 1 keyframe/sec from head.
        let opts = FilmstripOpts {
            cols: 8,
            frame_width: 120,
            duration: 0.0,
        };
        let a = filmstrip_args("in.mp4", &opts, "out.jpg");
        assert_eq!(flag_val(&a, "-skip_frame"), Some("nokey"));
        assert_eq!(flag_val(&a, "-threads"), Some("1"));
        assert_eq!(a.iter().filter(|s| *s == "-i").count(), 1);
        let vf = flag_val(&a, "-vf").unwrap();
        assert!(vf.contains("fps=1.000000"), "vf: {vf}");
        assert!(vf.contains("tile=8x1"), "vf: {vf}");
    }

    #[test]
    fn input_segment_builds_seek_input_duration_and_maps() {
        // Copy-all-streams Trim with audio kept.
        let a = input_segment("in.mp4", 2.5, 4.0, "0:v?", true);
        assert_eq!(flag_val(&a, "-ss"), Some("2.5"));
        assert_eq!(flag_val(&a, "-i"), Some("in.mp4"));
        assert_eq!(flag_val(&a, "-t"), Some("4"));
        // -ss precedes -i (fast keyframe input seek).
        let ss = a.iter().position(|s| s == "-ss").unwrap();
        let i = a.iter().position(|s| s == "-i").unwrap();
        assert!(ss < i, "-ss must come before -i");
        let maps: Vec<&String> = a
            .iter()
            .enumerate()
            .filter(|(idx, s)| *s == "-map" && a.get(idx + 1).is_some())
            .map(|(idx, _)| &a[idx + 1])
            .collect();
        assert_eq!(maps, vec!["0:v?", "0:a?"]);
    }

    #[test]
    fn input_segment_omits_audio_map_when_excluded() {
        // Dropping audio = omit the 0:a? map entirely (no -an needed for copy).
        let a = input_segment("in.mp4", 0.0, 1.0, "0:v:0", false);
        assert_eq!(flag_val(&a, "-map"), Some("0:v:0"));
        assert_eq!(a.iter().filter(|s| *s == "-map").count(), 1, "no audio map");
        assert!(!a.iter().any(|s| s == "0:a?"));
    }

    #[test]
    fn audio_args_m4a_copies_then_falls_back_to_aac() {
        let copy = audio_args("in.mp4", 1.0, 3.0, 1.0, false, true, "out.m4a");
        assert!(copy.contains(&"-vn".to_string()), "drops video");
        assert_eq!(flag_val(&copy, "-map"), Some("0:a:0"));
        assert_eq!(flag_val(&copy, "-c:a"), Some("copy"));
        assert_eq!(flag_val(&copy, "-movflags"), Some("+faststart"));
        assert_eq!(copy.last().unwrap(), "out.m4a");
        // copy=false → AAC re-encode for a non-AAC source.
        let aac = audio_args("in.mp4", 1.0, 3.0, 1.0, false, false, "out.m4a");
        assert_eq!(flag_val(&aac, "-c:a"), Some("aac"));
        assert_eq!(flag_val(&aac, "-b:a"), Some("192k"));
    }

    #[test]
    fn audio_args_mp3_always_reencodes() {
        let a = audio_args("in.mp4", 0.0, 2.0, 1.0, true, true, "out.mp3");
        assert!(a.contains(&"-vn".to_string()));
        assert_eq!(flag_val(&a, "-c:a"), Some("libmp3lame"));
        assert_eq!(flag_val(&a, "-q:a"), Some("2"));
        // MP3 never stream-copies even if copy=true was passed.
        assert!(!a.iter().any(|s| s == "copy"));
        assert_eq!(a.last().unwrap(), "out.mp3");
    }

    #[test]
    fn crop_filter_uses_ffmpeg_size_then_offset_order() {
        // ffmpeg order is crop=out_w:out_h:x:y, NOT x:y:w:h.
        assert_eq!(
            crop_filter(Some((320, 180, 1280, 720))),
            Some("crop=1280:720:320:180".to_string())
        );
        assert_eq!(crop_filter(None), None);
    }

    #[test]
    fn compose_vf_puts_crop_before_scale() {
        let crop = Some("crop=1280:720:0:0".to_string());
        let scale = Some("scale=-2:'min(ih,720)'".to_string());
        assert_eq!(
            compose_vf(crop.clone(), scale.clone()),
            Some("crop=1280:720:0:0,scale=-2:'min(ih,720)'".to_string())
        );
        assert_eq!(compose_vf(crop.clone(), None), crop);
        assert_eq!(compose_vf(None, scale.clone()), scale);
        assert_eq!(compose_vf(None, None), None);
    }

    #[test]
    fn gif_args_injects_setpts_for_speed_keeping_input_t() {
        let opts = GifOpts {
            fps: 15,
            width: 640,
            webp: false,
        };
        // 2x GIF: setpts leads the palette graph; -t stays an input-side read
        // of the source region (output naturally plays over dur/speed).
        let a = gif_args("in.mp4", 0.0, 4.0, 2.0, &opts, "out.gif");
        let filter = flag_val(&a, "-filter_complex").unwrap();
        assert!(
            filter.starts_with("setpts=0.500000*PTS,fps=15"),
            "filter: {filter}"
        );
        assert_eq!(flag_val(&a, "-t"), Some("4"));
        // 1x adds no setpts (byte-identical to the old graph).
        let b = gif_args("in.mp4", 0.0, 4.0, 1.0, &opts, "out.gif");
        assert!(!flag_val(&b, "-filter_complex").unwrap().contains("setpts"));
        // WebP path retimes too.
        let w = gif_args(
            "in.mp4",
            0.0,
            4.0,
            0.5,
            &GifOpts {
                fps: 24,
                width: 480,
                webp: true,
            },
            "out.webp",
        );
        assert!(flag_val(&w, "-vf")
            .unwrap()
            .starts_with("setpts=2.000000*PTS,fps=24"));
    }

    #[test]
    fn audio_args_speed_forces_reencode_with_atempo() {
        // 0.5x M4A: even with copy=true, atempo forces an AAC re-encode.
        let a = audio_args("in.mp4", 0.0, 4.0, 0.5, false, true, "out.m4a");
        assert_eq!(flag_val(&a, "-filter:a"), Some("atempo=0.500000"));
        assert_eq!(flag_val(&a, "-c:a"), Some("aac"));
        assert!(!a.iter().any(|s| s == "copy"), "speed must not stream-copy");
        // 4x chains atempo; MP3 keeps libmp3lame plus the filter.
        let m = audio_args("in.mp4", 0.0, 4.0, 4.0, true, true, "out.mp3");
        assert_eq!(
            flag_val(&m, "-filter:a"),
            Some("atempo=2.0,atempo=2.000000")
        );
        assert_eq!(flag_val(&m, "-c:a"), Some("libmp3lame"));
        // 1x is unchanged: copy still allowed, no audio filter.
        let c = audio_args("in.mp4", 0.0, 4.0, 1.0, false, true, "out.m4a");
        assert_eq!(flag_val(&c, "-c:a"), Some("copy"));
        assert!(!c.iter().any(|s| s == "-filter:a"));
    }

    #[test]
    fn speed_setpts_filter_inverts_speed_and_skips_unit() {
        // 2x faster → halve the timestamps.
        assert_eq!(
            speed_setpts_filter(2.0),
            Some("setpts=0.500000*PTS".to_string())
        );
        // 0.5x slower → double the timestamps.
        assert_eq!(
            speed_setpts_filter(0.5),
            Some("setpts=2.000000*PTS".to_string())
        );
        assert_eq!(
            speed_setpts_filter(4.0),
            Some("setpts=0.250000*PTS".to_string())
        );
        // 1x is a no-op so the lossless/normal path is untouched.
        assert_eq!(speed_setpts_filter(1.0), None);
    }

    #[test]
    fn atempo_chain_stays_within_per_stage_limits() {
        // In-range speeds are a single stage.
        assert_eq!(atempo_chain(1.5), Some("atempo=1.500000".to_string()));
        assert_eq!(atempo_chain(2.0), Some("atempo=2.000000".to_string()));
        assert_eq!(atempo_chain(0.5), Some("atempo=0.500000".to_string()));
        // Out-of-range speeds chain 2.0 / 0.5 stages plus the remainder.
        assert_eq!(
            atempo_chain(4.0),
            Some("atempo=2.0,atempo=2.000000".to_string())
        );
        assert_eq!(
            atempo_chain(0.25),
            Some("atempo=0.5,atempo=0.500000".to_string())
        );
        // Every stage stays within ffmpeg's 0.5..=2.0 atempo window.
        for &s in &[0.25_f64, 0.5, 0.75, 1.5, 2.0, 3.0, 4.0] {
            let chain = atempo_chain(s).unwrap();
            for stage in chain.split(',') {
                let v: f64 = stage.trim_start_matches("atempo=").parse().unwrap();
                assert!(
                    (0.5..=2.0).contains(&v),
                    "stage {stage} out of range for {s}x"
                );
            }
        }
        assert_eq!(atempo_chain(1.0), None);
    }

    #[test]
    fn input_segment_out_caps_output_at_retimed_duration() {
        // 0.5x of a 4 s region → 8 s of output; -ss/-i still cover the source.
        let a = input_segment_out("in.mp4", 2.0, 8.0, "0:v:0", true);
        assert_eq!(flag_val(&a, "-ss"), Some("2"));
        assert_eq!(flag_val(&a, "-i"), Some("in.mp4"));
        assert_eq!(flag_val(&a, "-t"), Some("8"));
        // -t stays AFTER -i (an output cap, not an input window).
        let i = a.iter().position(|s| s == "-i").unwrap();
        let t = a.iter().position(|s| s == "-t").unwrap();
        assert!(i < t, "-t must remain an output cap (after -i)");
        let maps: Vec<&String> = a
            .iter()
            .enumerate()
            .filter(|(idx, s)| *s == "-map" && a.get(idx + 1).is_some())
            .map(|(idx, _)| &a[idx + 1])
            .collect();
        assert_eq!(maps, vec!["0:v:0", "0:a?"]);
    }

    #[test]
    fn compose_vf_speed_appends_setpts_last() {
        let crop = Some("crop=1280:720:0:0".to_string());
        let scale = Some("scale=-2:'min(ih,720)'".to_string());
        let setpts = Some("setpts=0.500000*PTS".to_string());
        // crop, then scale, then setpts — pixel order preserved, retime last.
        assert_eq!(
            compose_vf_speed(crop.clone(), scale.clone(), setpts.clone()),
            Some("crop=1280:720:0:0,scale=-2:'min(ih,720)',setpts=0.500000*PTS".to_string())
        );
        // setpts alone (no crop/scale).
        assert_eq!(compose_vf_speed(None, None, setpts.clone()), setpts);
        // No retime → identical to compose_vf.
        assert_eq!(
            compose_vf_speed(crop.clone(), scale.clone(), None),
            compose_vf(crop, scale)
        );
        assert_eq!(compose_vf_speed(None, None, None), None);
    }

    #[test]
    fn waveform_args_decodes_mono_pcm_to_stdout() {
        let a = waveform_args("in.mp4", &WaveformOpts { sample_rate: 4000 });
        assert_eq!(flag_val(&a, "-i"), Some("in.mp4"));
        assert!(a.contains(&"-vn".to_string()), "should drop video");
        assert_eq!(flag_val(&a, "-ac"), Some("1"));
        assert_eq!(flag_val(&a, "-ar"), Some("4000"));
        assert_eq!(flag_val(&a, "-f"), Some("s16le"));
        assert_eq!(a.last().unwrap(), "pipe:1");
    }

    #[test]
    fn peaks_buckets_and_normalises_to_loudest() {
        // 4 buckets of 2 equal samples → RMS 0, 100, 0, 32767 → normalise to 32767.
        let samples: Vec<i16> = vec![0, 0, 100, 100, 0, 0, 32767, 32767];
        let p = peaks(&samples, 4);
        assert_eq!(p.len(), 4);
        assert_eq!(p[0], 0.0);
        assert_eq!(p[2], 0.0);
        assert!((p[3] - 1.0).abs() < 1e-6, "loudest bucket fills: {}", p[3]);
        assert!(
            (p[1] - 100.0 / 32767.0).abs() < 1e-4,
            "quiet bucket: {}",
            p[1]
        );
    }

    #[test]
    fn peaks_handles_silence_and_clipping_edges() {
        // Silence → all zeros (no divide-by-zero).
        assert_eq!(peaks(&[0, 0, 0, 0], 4), vec![0.0, 0.0, 0.0, 0.0]);
        // No samples (audio-less Clip) → flat waveform of the requested length.
        assert_eq!(peaks(&[], 3), vec![0.0, 0.0, 0.0]);
        // Negative clip is treated by magnitude; full-scale → 1.0.
        let p = peaks(&[i16::MIN, i16::MIN, 0, 0], 2);
        assert!(
            (p[0] - 1.0).abs() < 1e-6,
            "clipped negative fills: {}",
            p[0]
        );
        assert_eq!(p[1], 0.0);
        // More buckets than samples doesn't panic; extra buckets stay 0.
        let q = peaks(&[10_000, -10_000], 5);
        assert_eq!(q.len(), 5);
        assert!(q.iter().all(|&v| (0.0..=1.0).contains(&v)));
    }
}
