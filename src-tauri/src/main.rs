// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // WebKitGTK's DMA-BUF accelerated rendering black-screens the webview on
    // common Linux setups (NVIDIA proprietary drivers, some Wayland
    // compositors): the app runs but nothing paints. Disable that renderer
    // before the webview is created unless the user has explicitly chosen a
    // value themselves. If a black window is ever still reported, the next
    // lever is WEBKIT_DISABLE_COMPOSITING_MODE=1.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    klipt_lib::run()
}
