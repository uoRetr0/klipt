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

/// What we stash to restore the window when leaving our manual fullscreen: the
/// full `WINDOWPLACEMENT` (position, size, and normal/maximized state) plus the
/// `GWL_STYLE` snapshot so we can re-add the resize frame we strip on enter.
/// Process-global (one main window); `Some` exactly when we're in fullscreen, so
/// it doubles as the toggle's state.
#[cfg(windows)]
static FULLSCREEN: std::sync::Mutex<
    Option<(
        windows::Win32::UI::WindowsAndMessaging::WINDOWPLACEMENT,
        isize,
    )>,
> = std::sync::Mutex::new(None);

/// Toggle true, taskbar-covering fullscreen. Returns the new state (`true` =
/// now fullscreen) so the frontend tracks it directly.
///
/// We do NOT use tao's `set_fullscreen` (or its geometry helpers): this window is
/// undecorated *and resizable*, so it keeps a `WS_THICKFRAME`. tao's
/// `WM_NCCALCSIZE` handler hides that frame visually, but `set_size`/`set_position`
/// still place the window by its *outer* rect — which sits a few pixels past the
/// monitor on every edge (it even spills onto the neighbouring display). A rect
/// that doesn't exactly equal the monitor never triggers the shell's
/// full-screen-app detection, so the taskbar stays put.
///
/// The fix (the standard borderless-fullscreen path) goes straight to Win32:
/// strip `WS_THICKFRAME` so the outer rect == client rect, size the window to the
/// exact `rcMonitor` device rect, raise it topmost, and pull it foreground so the
/// shell hides the taskbar for it. Restoring re-adds the frame and replays the
/// saved `WINDOWPLACEMENT` (which round-trips position, size and maximized state
/// exactly — restoring via tao's `set_size` drifts by the resize-border width on
/// every toggle).
#[cfg(windows)]
#[tauri::command]
pub(crate) fn toggle_fullscreen(window: tauri::WebviewWindow) -> Result<bool, String> {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, HMONITOR, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, GetWindowPlacement, SetForegroundWindow, SetWindowLongPtrW,
        SetWindowPlacement, SetWindowPos, GWL_STYLE, HWND_NOTOPMOST, HWND_TOPMOST,
        SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, WINDOWPLACEMENT,
        WS_THICKFRAME,
    };

    let raw = window.hwnd().map_err(|e| e.to_string())?;
    let hwnd = HWND(raw.0 as _);
    let mut slot = FULLSCREEN.lock().map_err(|_| "fullscreen lock poisoned")?;

    if let Some((placement, style)) = slot.take() {
        // Exit: re-add the resize frame, drop topmost, then replay the exact
        // pre-fullscreen placement.
        unsafe {
            SetWindowLongPtrW(hwnd, GWL_STYLE, style);
            // Apply the style change (SWP_FRAMECHANGED) without moving/sizing yet.
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_NOTOPMOST),
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOOWNERZORDER,
            );
            let _ = SetWindowPlacement(hwnd, &placement);
        }
        window.set_always_on_top(false).map_err(|e| e.to_string())?;
        return Ok(false);
    }

    // Enter: snapshot the placement + style (the placement keeps the maximized
    // state for us), drop any maximized state so we can size freely, then cover
    // the monitor exactly.
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        let mut placement = WINDOWPLACEMENT {
            length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
            ..Default::default()
        };
        GetWindowPlacement(hwnd, &mut placement).map_err(|e| e.to_string())?;

        // Exact monitor bounds in device pixels (rcMonitor, not the work area).
        let hmon: HMONITOR = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(hmon, &mut mi).as_bool() {
            return Err("GetMonitorInfoW failed".into());
        }
        let rc: RECT = mi.rcMonitor;

        if window.is_maximized().map_err(|e| e.to_string())? {
            window.unmaximize().map_err(|e| e.to_string())?;
        }

        *slot = Some((placement, style));

        // Drop the invisible resize border so the outer rect equals the client
        // rect, then place the window on the exact monitor rect, topmost.
        SetWindowLongPtrW(hwnd, GWL_STYLE, style & !(WS_THICKFRAME.0 as isize));
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            rc.left,
            rc.top,
            rc.right - rc.left,
            rc.bottom - rc.top,
            SWP_FRAMECHANGED | SWP_NOOWNERZORDER,
        )
        .map_err(|e| e.to_string())?;
        // Foreground so the shell's full-screen-app detection hides the taskbar.
        let _ = SetForegroundWindow(hwnd);
    }
    window.set_always_on_top(true).map_err(|e| e.to_string())?;
    Ok(true)
}

/// Non-Windows fallback: tao's native fullscreen is fine off Windows (the app
/// ships Windows-only, but this keeps the crate cross-compilable).
#[cfg(not(windows))]
#[tauri::command]
pub(crate) fn toggle_fullscreen(window: tauri::WebviewWindow) -> Result<bool, String> {
    let now = !window.is_fullscreen().map_err(|e| e.to_string())?;
    window.set_fullscreen(now).map_err(|e| e.to_string())?;
    Ok(now)
}
