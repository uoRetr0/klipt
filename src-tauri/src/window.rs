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
/// window's `resizable` flag, which we clear on enter to drop the invisible
/// `WS_THICKFRAME` resize border and re-set on exit.
/// Process-global (one main window); `Some` exactly when we're in fullscreen, so
/// it doubles as the toggle's state.
#[cfg(windows)]
static FULLSCREEN: std::sync::Mutex<
    Option<(
        windows::Win32::UI::WindowsAndMessaging::WINDOWPLACEMENT,
        bool,
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
/// drop the resize frame via tao's `set_resizable(false)` so the outer rect ==
/// client rect (a raw `SetWindowLongPtrW` strip doesn't stick — tao re-applies
/// `WS_THICKFRAME` on its next message, leaving a ~7px transparent border that
/// shows the desktop and defeats the shell's full-screen detection), size the
/// window to the exact `rcMonitor` device rect, raise it topmost, and pull it
/// foreground so the shell hides the taskbar for it. Restoring re-adds the frame
/// (`set_resizable(true)`) and replays the
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
        GetWindowInfo, GetWindowPlacement, SetForegroundWindow, SetWindowPlacement, SetWindowPos,
        HWND_NOTOPMOST, HWND_TOPMOST, SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE,
        WINDOWINFO, WINDOWPLACEMENT,
    };

    let raw = window.hwnd().map_err(|e| e.to_string())?;
    let hwnd = HWND(raw.0 as _);
    let mut slot = FULLSCREEN.lock().map_err(|_| "fullscreen lock poisoned")?;

    if let Some((placement, was_resizable)) = slot.take() {
        // Exit: drop topmost, replay the exact pre-fullscreen placement, then
        // re-add the resize frame via tao (so its window flags stay in sync).
        unsafe {
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
        window
            .set_resizable(was_resizable)
            .map_err(|e| e.to_string())?;
        window.set_always_on_top(false).map_err(|e| e.to_string())?;
        return Ok(false);
    }

    // Enter: snapshot the placement (it keeps the maximized state for us) and the
    // resizable flag *before* touching the window, drop any maximized state so we
    // can size freely, clear the resize frame, then cover the monitor exactly.
    let was_resizable = window.is_resizable().map_err(|e| e.to_string())?;
    let mut placement = WINDOWPLACEMENT {
        length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
        ..Default::default()
    };
    unsafe { GetWindowPlacement(hwnd, &mut placement).map_err(|e| e.to_string())? };

    if window.is_maximized().map_err(|e| e.to_string())? {
        window.unmaximize().map_err(|e| e.to_string())?;
    }
    // tao-aware frame removal: clears WS_THICKFRAME *and* updates tao's internal
    // window flags, so it isn't re-applied on the next message (a raw
    // SetWindowLongPtrW strip is reverted by tao, leaving a transparent border).
    window.set_resizable(false).map_err(|e| e.to_string())?;

    unsafe {
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

        *slot = Some((placement, was_resizable));

        // Place the window on the exact monitor rect, topmost.
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

        // The webview fills this window's CLIENT rect, but an undecorated tao
        // window can still report a few px of non-client border even with the
        // resize frame gone — that border paints nothing, so the desktop shows
        // through at the edges. Measure whatever border remains (GetWindowInfo
        // reports both rects in screen coords) and grow the outer window by
        // exactly that, so the client rect lands flush on the monitor; the
        // leftover frame hangs off-screen and is clipped.
        let mut wi = WINDOWINFO {
            cbSize: std::mem::size_of::<WINDOWINFO>() as u32,
            ..Default::default()
        };
        GetWindowInfo(hwnd, &mut wi).map_err(|e| e.to_string())?;
        let bl = wi.rcClient.left - wi.rcWindow.left; // left border
        let bt = wi.rcClient.top - wi.rcWindow.top; // top border
        let br = wi.rcWindow.right - wi.rcClient.right; // right border
        let bb = wi.rcWindow.bottom - wi.rcClient.bottom; // bottom border
        if bl != 0 || bt != 0 || br != 0 || bb != 0 {
            SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                rc.left - bl,
                rc.top - bt,
                (rc.right - rc.left) + bl + br,
                (rc.bottom - rc.top) + bt + bb,
                SWP_FRAMECHANGED | SWP_NOOWNERZORDER,
            )
            .map_err(|e| e.to_string())?;
        }

        // Foreground so the shell's full-screen-app detection hides the taskbar.
        let _ = SetForegroundWindow(hwnd);
    }
    window.set_always_on_top(true).map_err(|e| e.to_string())?;
    Ok(true)
}

/// Non-Windows fallback: tao's native `set_fullscreen` is fine off Windows —
/// the Win32 path above only exists because tao's undecorated-but-resizable
/// window geometry fights the Windows shell's full-screen-app detection.
#[cfg(not(windows))]
#[tauri::command]
pub(crate) fn toggle_fullscreen(window: tauri::WebviewWindow) -> Result<bool, String> {
    let now = !window.is_fullscreen().map_err(|e| e.to_string())?;
    window.set_fullscreen(now).map_err(|e| e.to_string())?;
    Ok(now)
}
