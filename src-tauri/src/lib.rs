//! Klipt's Tauri backend. The work is split across focused modules:
//!   * `ffmpeg`   — bundled-sidecar plumbing + banner / progress parsing
//!   * `naming`   — output path / name resolution (pure)
//!   * `media`    — encode argument builders + bitrate / waveform math (pure)
//!   * `settings` — persisted preferences + the get/set commands
//!   * `library`  — watched-folder scanning
//!   * `commands` — the Trim / Compress / GIF / thumbnail / file-op commands
//!   * `window`   — borderless-window chrome + maximize toggle
//!
//! This file just declares those modules and wires the command handlers.

mod commands;
mod ffmpeg;
mod library;
mod media;
mod naming;
mod settings;
mod window;

use commands::{
    clip_filmstrip, clip_thumbnail, clip_waveform, compress_clip, delete_clip, gif_clip,
    probe_clip, rename_clip, restore_clip, trim_clip,
};
use library::list_recent_clips;
use settings::{get_settings, set_settings};
use window::toggle_maximize;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // WebKitGTK's DMA-BUF renderer paints a black window on many Linux GPU/driver
    // combos (notably NVIDIA, and WebKitGTK >= 2.42). Disable it before the webview
    // is created so Klipt renders out of the box; an explicit env override still wins.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|_app| {
            #[cfg(windows)]
            {
                use tauri::Manager;
                if let Some(window) = _app.get_webview_window("main") {
                    if let Err(e) = window::refine_window_chrome(&window) {
                        eprintln!("failed to refine window chrome: {e}");
                    }
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            probe_clip,
            trim_clip,
            compress_clip,
            gif_clip,
            list_recent_clips,
            clip_thumbnail,
            clip_waveform,
            clip_filmstrip,
            delete_clip,
            restore_clip,
            rename_clip,
            get_settings,
            set_settings,
            toggle_maximize
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
