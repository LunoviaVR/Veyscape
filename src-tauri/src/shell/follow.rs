//! Keeps a torn-off tab's new window under the pointer while the tab is still being dragged.
//!
//! The drag itself runs in WebView2's browser process, so this window is moved from a background
//! thread (asynchronous window positioning) until the mouse button is released. Over another
//! browser window's tab strip the new window hides, so the drop lands in that strip and the tab
//! joins it (the emptied new window then closes). If it is still around after the release, it
//! is shown and brought to the front.

use crate::browser::{self, WindowId};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetSystemMetrics, GetWindowRect, IsWindow, IsWindowVisible, SetForegroundWindow,
    SetWindowPos, ShowWindowAsync, SM_SWAPBUTTON, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOSIZE,
    SWP_NOZORDER, SWP_SHOWWINDOW, SW_HIDE, SW_SHOWNA,
};

/// Height of the tab strip in CSS px (the window's top band that accepts dropped tabs).
const STRIP_HEIGHT: f64 = 40.0;

/// Window handles cross threads as plain numbers (`HWND` is not `Send`).
fn hwnd(window: &tauri::Window) -> Option<isize> {
    window.hwnd().ok().map(|h| h.0 as isize)
}

/// Follows the pointer with window `id` until the button is released. Blocking: run it on a
/// background thread.
pub fn follow_pointer(app: &AppHandle, id: WindowId, grab: (f64, f64)) {
    // The window is being created; wait for it.
    let deadline = Instant::now() + Duration::from_secs(3);
    let window = loop {
        if let Some(w) = app.get_window(&browser::window_label(id)) {
            break w;
        }
        if Instant::now() > deadline {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let (Some(raw), Ok(scale)) = (hwnd(&window), window.scale_factor()) else {
        return;
    };
    // Tab strips of the other browser windows, in screen pixels.
    let strips: Vec<RECT> = app
        .windows()
        .into_iter()
        .filter(|(label, _)| browser::window_id(label).is_some_and(|w| w != id))
        .filter_map(|(_, w)| {
            let h = HWND(hwnd(&w)? as _);
            let s = w.scale_factor().ok()?;
            let mut r = RECT::default();
            unsafe { GetWindowRect(h, &mut r).ok()? };
            unsafe { IsWindowVisible(h) }.as_bool().then_some(())?;
            r.bottom = r.top + (STRIP_HEIGHT * s) as i32;
            Some(r)
        })
        .collect();
    let floating = HWND(raw as _);
    let button = match unsafe { GetSystemMetrics(SM_SWAPBUTTON) } {
        0 => VK_LBUTTON,
        _ => VK_RBUTTON,
    };
    let held = || unsafe { GetAsyncKeyState(button.0 as i32) } < 0;
    let (gx, gy) = ((grab.0 * scale) as i32, (grab.1 * scale) as i32);
    let mut hidden = false;
    let give_up = Instant::now() + Duration::from_secs(120);
    while held() && Instant::now() < give_up {
        let mut p = POINT::default();
        if unsafe { GetCursorPos(&mut p) }.is_ok() {
            let over_strip = strips
                .iter()
                .any(|r| p.x >= r.left && p.x < r.right && p.y >= r.top && p.y < r.bottom);
            unsafe {
                if over_strip {
                    if !hidden {
                        let _ = ShowWindowAsync(floating, SW_HIDE);
                        hidden = true;
                    }
                } else {
                    let mut flags = SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS;
                    if hidden {
                        flags |= SWP_SHOWWINDOW;
                        hidden = false;
                    }
                    let _ = SetWindowPos(floating, None, p.x - gx, p.y - gy, 0, 0, flags);
                }
            }
        }
        std::thread::sleep(Duration::from_millis(8));
    }
    if hidden {
        // Dropped on another window's strip: the tab moved there and this window closes. If the
        // drop did not take, the window (and its tab) must not stay invisible.
        std::thread::sleep(Duration::from_millis(800));
        if !unsafe { IsWindow(Some(floating)) }.as_bool() {
            return;
        }
        unsafe {
            let _ = ShowWindowAsync(floating, SW_SHOWNA);
        }
    }
    unsafe {
        let _ = SetForegroundWindow(floating);
    }
}
