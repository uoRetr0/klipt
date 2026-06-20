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
    opts: &GifOpts,
    output: &str,
) -> Vec<String> {
    let fps = opts.fps.clamp(1, 50);
    let width = opts.width.clamp(64, 1920);
    let scale = format!("scale={width}:-1:flags=lanczos");
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
            format!("fps={fps},{scale}"),
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
            "fps={fps},{scale},split[s0][s1];[s0]palettegen=stats_mode=diff[p];\
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
        let a = gif_args("in.mp4", 2.5, 4.0, &opts, "out.gif");
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
        let a = gif_args("in.mp4", 0.0, 3.0, &opts, "out.webp");
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
        let a = gif_args("in.mp4", 0.0, 1.0, &opts, "out.gif");
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
