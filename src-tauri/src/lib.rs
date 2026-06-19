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
    // WebKitGTK's DMA-BUF renderer is the fast path for compositing and <video>,
    // but it renders a black window on some GPU/driver combos. We default it OFF
    // for out-of-the-box reliability. On a healthy GPU stack (most Mesa/AMD/Intel)
    // it's faster and fixes laggy fullscreen + video — opt back in with KLIPT_GPU=1.
    // An explicit WEBKIT_DISABLE_DMABUF_RENDERER in the environment always wins.
    #[cfg(target_os = "linux")]
    {
        let force_gpu = matches!(std::env::var("KLIPT_GPU").as_deref(), Ok("1"));
        if !force_gpu && std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // The static assetProtocol scope is empty; grant the asset protocol
            // access to Klipt's own dirs (cache + watched/output folders) so
            // thumbnails and clip playback resolve without opening the whole disk.
            let settings = settings::read_settings(app.handle());
            settings::grant_asset_scope(app.handle(), &settings);

            #[cfg(windows)]
            {
                use tauri::Manager;
                if let Some(window) = app.get_webview_window("main") {
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
