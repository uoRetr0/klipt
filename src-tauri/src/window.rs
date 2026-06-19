//! Borderless-window chrome tweaks: a subtle dark frame border on Windows 11 and
//! a native maximize toggle that respects the taskbar.

/// Soften the borderless window's frame on Windows 11. Without decorations,
/// Windows draws a bright default hairline border around the window; swap it for
/// a subtle dark line that matches the app's `--border` (#26262b) so the window
/// edge reads as a quiet seam rather than a white outline.
#[cfg(windows)]
pub(crate) fn refine_window_chrome(window: &tauri::WebviewWindow) -> Result<(), String> {
    use windows::Win32::Foundation::{COLORREF, HWND};
    use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_BORDER_COLOR};

    // Rebuild the HWND from the raw pointer so this is independent of whichever
    // `windows` version Tauri itself links against.
    let raw = window.hwnd().map_err(|e| e.to_string())?;
    let hwnd = HWND(raw.0 as _);
    // COLORREF is 0x00BBGGRR; #26262b -> R=26 G=26 B=2b.
    let color = COLORREF(0x002B2626);
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            &color as *const _ as *const core::ffi::c_void,
            std::mem::size_of::<COLORREF>() as u32,
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Toggle native maximize for the borderless window. tao's window proc clamps a
/// maximized undecorated window's client rect to the monitor work area (see its
/// `WM_NCCALCSIZE` handler), so this fills the screen without covering the
/// taskbar and without the invisible-frame offset that hand-positioning hits.
#[tauri::command]
pub(crate) fn toggle_maximize(window: tauri::WebviewWindow) -> Result<(), String> {
    if window.is_maximized().map_err(|e| e.to_string())? {
        window.unmaximize().map_err(|e| e.to_string())
    } else {
        window.maximize().map_err(|e| e.to_string())
    }
}
