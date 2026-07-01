//! In-process libav filmstrip decode: open the Clip once and seek-decode only
//! the `cols` keyframes the sprite needs, instead of the sidecar decoding every
//! keyframe across the whole Clip (~600 on a short-GOP ShadowPlay clip). It is
//! CPU-based but seek-smart, so it stays fast on short-GOP clips and needs no
//! GPU. Decode-only: frames are scaled (swscale) and the assembled RGB strip is
//! JPEG-encoded in-process — no avcodec encoder / muxer, so the FFI surface (and
//! the bundled LGPL DLLs) stay decode-only. Any failure returns `Err` and the
//! caller falls back to the sidecar path (see `clip_filmstrip`).

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use ffmpeg::format::Pixel;
use ffmpeg::media::Type;
use ffmpeg::software::scaling::{context::Context as Scaler, flag::Flags};
use ffmpeg_the_third as ffmpeg;

/// Set once libav proves globally unusable (its one-time `init` failed) so we
/// stop trying it and go straight to the sidecar. Per-clip decode errors do NOT
/// trip this — a single bad Clip must not disable the fast path for good ones
/// (cf. `NVDEC_DISABLED`). Process-global; resets on app restart.
pub(crate) static LIBAV_DISABLED: AtomicBool = AtomicBool::new(false);

/// FFmpeg's internal timestamp base for stream-independent seeks (microseconds).
const AV_TIME_BASE: f64 = 1_000_000.0;

/// The presentation time (seconds) of filmstrip cell `i` of `cols`, sampled
/// evenly across `duration`. Matches the sidecar's `fps=cols/duration` filter
/// (frames at 0, d/cols, 2d/cols, …) so the libav strip lines up cell-for-cell
/// with the CPU/GPU sprite the frontend's `frameIndexAt` mapping expects. Pure.
fn sample_time(i: u32, cols: u32, duration: f64) -> f64 {
    (i as f64) * duration / (cols.max(1) as f64)
}

/// Render a `cols`-cell filmstrip sprite for `input` to `output` (JPEG), each
/// cell `frame_width` wide, sampled evenly across `duration` seconds. Opens the
/// file once and seeks to the keyframe at/just before each sample point, so the
/// cost scales with `cols` (~24) rather than the Clip's keyframe count (~600).
///
/// Sample points match the sidecar's `fps=cols/duration` filter (`t = i *
/// duration / cols`) so the sprite lines up cell-for-cell with the CPU/GPU
/// output the frontend's `frameIndexAt` mapping expects.
pub(crate) fn filmstrip_libav(
    input: &str,
    cols: u32,
    frame_width: u32,
    duration: f64,
    output: &str,
) -> Result<(), String> {
    if LIBAV_DISABLED.load(Ordering::Relaxed) {
        return Err("libav disabled".into());
    }
    if duration <= 0.0 {
        // No duration -> can't place samples evenly; let the sidecar handle it.
        return Err("libav: unknown duration".into());
    }
    ffmpeg::init().map_err(|e| {
        LIBAV_DISABLED.store(true, Ordering::Relaxed);
        format!("libav init failed: {e}")
    })?;

    let cols = cols.max(1);
    let mut ictx =
        ffmpeg::format::input(Path::new(input)).map_err(|e| format!("libav open: {e}"))?;

    // Extract the video stream's index + a decoder, then drop the stream borrow
    // so `ictx` is free for the mutable seek/read loop below.
    let (stream_index, mut decoder, src_w, src_h, pix) = {
        let stream = ictx
            .streams()
            .best(Type::Video)
            .ok_or("libav: no video stream")?;
        let idx = stream.index();
        let decoder = ffmpeg::codec::context::Context::from_parameters(stream.parameters())
            .map_err(|e| format!("libav decoder ctx: {e}"))?
            .decoder()
            .video()
            .map_err(|e| format!("libav decoder: {e}"))?;
        let (w, h, p) = (decoder.width(), decoder.height(), decoder.format());
        (idx, decoder, w, h, p)
    };
    if src_w == 0 || src_h == 0 {
        return Err("libav: zero dimensions".into());
    }

    // scale=frame_width:-2 -> keep aspect, snap height to even (mjpeg/yuv420-safe).
    let out_w = frame_width;
    let mut out_h = ((frame_width as f64) * (src_h as f64) / (src_w as f64)).round() as u32;
    out_h = (out_h / 2) * 2;
    if out_h < 2 {
        out_h = 2;
    }

    let mut scaler = Scaler::get(
        pix,
        src_w,
        src_h,
        Pixel::RGB24,
        out_w,
        out_h,
        Flags::LANCZOS,
    )
    .map_err(|e| format!("libav scaler: {e}"))?;

    let tile_w = out_w * cols;
    let row_bytes = (tile_w * 3) as usize; // packed RGB24, no padding in our canvas
    let cell_bytes = (out_w * 3) as usize;
    let mut canvas = vec![0u8; row_bytes * out_h as usize];

    for i in 0..cols {
        let t = sample_time(i, cols, duration);
        let ts = (t * AV_TIME_BASE) as i64;
        // Seek to the keyframe at/just before `t` (max_ts = ts -> never overshoot).
        let _ = ictx.seek(ts, ..=ts);
        decoder.flush();

        // Feed packets from this stream until the decoder emits the keyframe.
        let mut frame = ffmpeg::frame::Video::empty();
        let mut got = false;
        for res in ictx.packets() {
            let (s, packet) = match res {
                Ok(sp) => sp,
                Err(_) => break, // demux error / end of stream for this seek
            };
            if s.index() != stream_index {
                continue;
            }
            if decoder.send_packet(&packet).is_err() {
                continue;
            }
            if decoder.receive_frame(&mut frame).is_ok() {
                got = true;
                break;
            }
        }
        if !got {
            continue; // leave this cell black rather than fail the whole strip
        }

        let mut rgb = ffmpeg::frame::Video::empty();
        scaler
            .run(&frame, &mut rgb)
            .map_err(|e| format!("libav scale: {e}"))?;

        // Blit the scaled cell into the canvas at column `i`, skipping swscale's
        // row padding (stride may exceed the packed cell width).
        let stride = rgb.stride(0);
        let data = rgb.data(0);
        let x_off = (i * out_w * 3) as usize;
        for y in 0..out_h as usize {
            let src = &data[y * stride..y * stride + cell_bytes];
            let dst = x_off + y * row_bytes;
            canvas[dst..dst + cell_bytes].copy_from_slice(src);
        }
    }

    // Encode the assembled strip to JPEG (quality ~ the sidecar's -q:v 4).
    jpeg_encoder::Encoder::new_file(output, 88)
        .map_err(|e| format!("libav jpeg: {e}"))?
        .encode(
            &canvas,
            tile_w as u16,
            out_h as u16,
            jpeg_encoder::ColorType::Rgb,
        )
        .map_err(|e| format!("libav jpeg encode: {e}"))?;

    if !Path::new(output).exists() {
        return Err("libav: output missing".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_time_spreads_cells_across_the_clip() {
        let (cols, dur) = (24, 180.0);
        // First cell at the head, evenly spaced by duration/cols, last before the end.
        assert_eq!(sample_time(0, cols, dur), 0.0);
        assert!((sample_time(1, cols, dur) - 7.5).abs() < 1e-9);
        assert!((sample_time(12, cols, dur) - 90.0).abs() < 1e-9);
        assert!(sample_time(cols - 1, cols, dur) < dur);
    }

    #[test]
    fn sample_time_survives_degenerate_cols() {
        // cols == 0 must not divide by zero (clamped to 1).
        assert_eq!(sample_time(0, 0, 120.0), 0.0);
    }
}
