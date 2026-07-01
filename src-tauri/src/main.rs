// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// Whether the NVIDIA proprietary driver is loaded. The driver exposes these
/// nodes whenever it's active; Mesa/AMD/Intel systems have neither.
#[cfg(target_os = "linux")]
fn nvidia_driver_present() -> bool {
    std::path::Path::new("/proc/driver/nvidia/version").exists()
        || std::path::Path::new("/dev/nvidia0").exists()
}

fn main() {
    // WebKitGTK's DMA-BUF accelerated rendering black-screens the webview on
    // NVIDIA-proprietary-driver setups (especially Wayland): the app runs but
    // nothing paints. Disabling it fixes those — but forcing it off everywhere
    // regressed AMD/Mesa in a past attempt (laggy fullscreen, broken video),
    // so the workaround applies only when the NVIDIA driver is actually
    // present, and never overrides a value the user set themselves. If a black
    // window is still reported elsewhere, the next lever is
    // WEBKIT_DISABLE_COMPOSITING_MODE=1.
    #[cfg(target_os = "linux")]
    if nvidia_driver_present() && std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    klipt_lib::run()
}
