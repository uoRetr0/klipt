//! Bundled-FFmpeg sidecar plumbing: running the process (plain + progress-
//! streamed), and parsing the bits of its banner / `-progress` output the rest
//! of the app needs (duration, dimensions, completion fraction).

use tauri::AppHandle;
use tauri_plugin_shell::process::Output;
use tauri_plugin_shell::ShellExt;

pub(crate) async fn run_ffmpeg(app: &AppHandle, args: Vec<String>) -> Result<Output, String> {
    app.shell()
        .sidecar("klipt-ffmpeg")
        .map_err(|e| e.to_string())?
        .args(args)
        .output()
        .await
        .map_err(|e| e.to_string())
}

/// Outcome of a streamed ffmpeg run — mirrors the bits of `Output` the encode
/// paths need (success + captured stderr for error reporting).
pub(crate) struct RunResult {
    pub(crate) success: bool,
    pub(crate) stderr: String,
}

/// Run ffmpeg while streaming progress to the frontend. Spawns the sidecar
/// (rather than awaiting `.output()`) so `-progress` lines can be read live;
/// each parsed fraction is mapped into `[base, base+span]` and emitted on the
/// `compress-progress` event. This lets a multi-pass encode map pass 1 → 0–50%
/// and pass 2 → 50–100%. stderr is captured for error messages; stdout carries
/// the machine-readable progress keys (`-progress pipe:1`).
pub(crate) async fn run_ffmpeg_progress(
    app: &AppHandle,
    args: Vec<String>,
    total_secs: f64,
    base: f64,
    span: f64,
) -> Result<RunResult, String> {
    use tauri::Emitter;
    use tauri_plugin_shell::process::CommandEvent;

    // Prepend progress reporting to stdout and silence the periodic stderr
    // stats line so only real diagnostics land in `stderr`.
    let mut full: Vec<String> = vec!["-progress".into(), "pipe:1".into(), "-nostats".into()];
    full.extend(args);

    let (mut rx, _child) = app
        .shell()
        .sidecar("klipt-ffmpeg")
        .map_err(|e| e.to_string())?
        .args(full)
        .spawn()
        .map_err(|e| e.to_string())?;

    let mut stderr = String::new();
    let mut success = false;
    while let Some(event) = rx.recv().await {
        match event {
            CommandEvent::Stdout(bytes) => {
                let chunk = String::from_utf8_lossy(&bytes);
                for line in chunk.lines() {
                    if let Some(frac) = parse_progress(line, total_secs) {
                        let p = (base + frac * span).clamp(0.0, 1.0);
                        let _ = app.emit("compress-progress", p);
                    }
                }
            }
            CommandEvent::Stderr(bytes) => {
                stderr.push_str(&String::from_utf8_lossy(&bytes));
            }
            CommandEvent::Terminated(payload) => {
                success = payload.code == Some(0);
            }
            _ => {}
        }
    }
    Ok(RunResult { success, stderr })
}

/// Parse ffmpeg's `-i` stderr banner for a Clip's duration (seconds) and the
/// first video stream's pixel dimensions. ffmpeg prints these lines in English
/// regardless of system locale. Any field that can't be found stays 0.
pub(crate) fn parse_ffmpeg_probe(stderr: &str) -> (f64, u32, u32, f64) {
    let mut duration = 0.0;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut fps = 0.0;

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
    // Frame rate appears later on the same line as "<n> fps" (e.g. "60 fps",
    // "59.94 fps"), so scan the tokens for both dimensions and the rate.
    if let Some(line) = stderr.lines().find(|l| l.contains("Video:")) {
        let toks: Vec<&str> = line.split([' ', ',']).filter(|s| !s.is_empty()).collect();
        for (i, tok) in toks.iter().enumerate() {
            if width == 0 {
                if let Some((w, h)) = tok.split_once('x') {
                    if let (Ok(w), Ok(h)) = (w.parse::<u32>(), h.parse::<u32>()) {
                        if w >= 16 && h >= 16 {
                            width = w;
                            height = h;
                        }
                    }
                }
            }
            if *tok == "fps" && i > 0 {
                if let Ok(v) = toks[i - 1].parse::<f64>() {
                    fps = v;
                }
            }
        }
    }

    (duration, width, height, fps)
}

/// Parse "HH:MM:SS.ffff" into seconds. Returns None for "N/A" or malformed input.
fn parse_hms(s: &str) -> Option<f64> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 3 {
        return None;
    }
    let h = parts[0].parse::<f64>().ok()?;
    let m = parts[1].parse::<f64>().ok()?;
    let sec = parts[2].parse::<f64>().ok()?;
    Some(h * 3600.0 + m * 60.0 + sec)
}

/// Parse one line of ffmpeg's `-progress` output into an overall completion
/// fraction in `[0, 1]`. Recognises `out_time_us=` (microseconds) and the
/// human-readable `out_time=HH:MM:SS.ffff`; every other key returns None. The
/// early negative-sentinel timestamp clamps to 0, an overshoot clamps to 1, and
/// a non-positive `total_secs` yields None (can't compute a fraction). Pure.
// `!(total_secs > 0.0)` deliberately also rejects NaN (see prepare_output).
#[allow(clippy::neg_cmp_op_on_partial_ord)]
fn parse_progress(line: &str, total_secs: f64) -> Option<f64> {
    if !(total_secs > 0.0) {
        return None;
    }
    let line = line.trim();
    let secs = if let Some(v) = line.strip_prefix("out_time_us=") {
        v.trim().parse::<f64>().ok().map(|us| us / 1_000_000.0)
    } else if let Some(v) = line.strip_prefix("out_time=") {
        parse_hms(v.trim())
    } else {
        None
    }?;
    Some((secs / total_secs).clamp(0.0, 1.0))
}

/// Probe a Clip's duration (seconds) and first-video-stream dimensions using
/// ffmpeg's `-i` banner. ffmpeg exits non-zero when given no output file but
/// prints the stream info to stderr first, which is what we parse.
/// Best-effort: returns zeros on any failure.
pub(crate) async fn ffmpeg_probe(app: &AppHandle, path: &str) -> (f64, u32, u32, f64) {
    match run_ffmpeg(
        app,
        vec!["-hide_banner".into(), "-i".into(), path.to_string()],
    )
    .await
    {
        Ok(out) => parse_ffmpeg_probe(&String::from_utf8_lossy(&out.stderr)),
        Err(_) => (0.0, 0, 0, 0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_progress_reads_out_time_us_and_clamps() {
        // Half-way through a 10s encode.
        let p = parse_progress("out_time_us=5000000", 10.0).unwrap();
        assert!((p - 0.5).abs() < 1e-9, "p was {p}");
        // Overshoot clamps to 1.0; the early negative sentinel clamps to 0.0.
        assert_eq!(parse_progress("out_time_us=20000000", 10.0), Some(1.0));
        assert_eq!(
            parse_progress("out_time_us=-9223372036854775807", 10.0),
            Some(0.0)
        );
    }

    #[test]
    fn parse_progress_reads_human_out_time() {
        let p = parse_progress("out_time=00:00:05.000000", 10.0).unwrap();
        assert!((p - 0.5).abs() < 1e-9, "p was {p}");
        let p2 = parse_progress("out_time=00:01:00.000000", 120.0).unwrap();
        assert!((p2 - 0.5).abs() < 1e-9, "p2 was {p2}");
    }

    #[test]
    fn parse_progress_ignores_other_keys_and_zero_total() {
        // Non-timestamp keys carry no fraction.
        assert_eq!(parse_progress("frame=42", 10.0), None);
        assert_eq!(parse_progress("progress=continue", 10.0), None);
        assert_eq!(parse_progress("out_time=N/A", 10.0), None);
        // Without a known duration there's no fraction to compute.
        assert_eq!(parse_progress("out_time_us=5000000", 0.0), None);
    }

    #[test]
    fn parse_progress_rejects_nan_and_negative_total() {
        // `!(total_secs > 0.0)` rejects NaN (NaN > 0.0 is false) and negatives,
        // so a bogus duration never yields a fraction.
        assert_eq!(parse_progress("out_time_us=5000000", f64::NAN), None);
        assert_eq!(parse_progress("out_time_us=5000000", -10.0), None);
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
        let (d, w, h, fps) = parse_ffmpeg_probe(stderr);
        assert!((d - 154.56).abs() < 0.01, "duration was {d}");
        assert_eq!((w, h), (1920, 1080));
        assert!((fps - 60.0).abs() < 0.01, "fps was {fps}");
    }

    #[test]
    fn parse_ffmpeg_probe_handles_missing_or_na_fields() {
        // No Duration / no Video line -> all zeros, no panic.
        assert_eq!(
            parse_ffmpeg_probe("garbage with no fields"),
            (0.0, 0, 0, 0.0)
        );
        // Duration N/A parses to 0 but dimensions + fps still read.
        let s = "  Duration: N/A, bitrate: N/A\n  Stream #0:0: Video: h264, 1280x720, 30 fps\n";
        let (d, w, h, fps) = parse_ffmpeg_probe(s);
        assert_eq!(d, 0.0);
        assert_eq!((w, h), (1280, 720));
        assert_eq!(fps, 30.0);
    }
}
