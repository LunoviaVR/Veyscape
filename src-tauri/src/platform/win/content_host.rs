//! The content host: a plain child HWND that parents every tab's WebView2 controller.
//! It sits between the shell webview (below) and the overlay webview (above) in z-order,
//! so popups drawn by the trusted overlay can cover web content.

use crate::errors::AppResult;
use crate::platform::Rect;
use std::sync::Once;
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

const CLASS: windows::core::PCWSTR = w!("VeyscapeContentHost");

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

pub fn create(parent: HWND) -> AppResult<HWND> {
    static REGISTER: Once = Once::new();
    unsafe {
        let instance = GetModuleHandleW(None)?;
        REGISTER.call_once(|| {
            let class = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(wndproc),
                hInstance: instance.into(),
                lpszClassName: CLASS,
                ..Default::default()
            };
            RegisterClassExW(&class);
        });
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            CLASS,
            None,
            WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
            0,
            0,
            0,
            0,
            Some(parent),
            None,
            Some(instance.into()),
            None,
        )?;
        Ok(hwnd)
    }
}

pub fn set_bounds(hwnd: HWND, r: Rect) {
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            None,
            r.x,
            r.y,
            r.width.max(0),
            r.height.max(0),
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

/// Clips the host to `rects` (host coordinates); `None` removes the clip.
pub fn set_region(hwnd: HWND, rects: Option<&[Rect]>) {
    use windows::Win32::Graphics::Gdi::{
        CombineRgn, CreateRectRgn, DeleteObject, SetWindowRgn, RGN_OR,
    };
    unsafe {
        let Some(rects) = rects else {
            SetWindowRgn(hwnd, None, true);
            return;
        };
        let region = CreateRectRgn(0, 0, 0, 0);
        for r in rects {
            let part = CreateRectRgn(r.x, r.y, r.x + r.width, r.y + r.height);
            CombineRgn(Some(region), Some(region), Some(part), RGN_OR);
            let _ = DeleteObject(part.into());
        }
        // The window owns the region after this call.
        if SetWindowRgn(hwnd, Some(region), true) == 0 {
            let _ = DeleteObject(region.into());
        }
    }
}

/// Registers a window's overlay webview (its container HWND) with that window's content host,
/// which then always stacks directly beneath it. Kept in the host's user data: one per window.
pub fn set_overlay_window(overlay: isize) {
    unsafe {
        let overlay = HWND(overlay as *mut _);
        let root = GetAncestor(overlay, GA_ROOT);
        if let Ok(host) = FindWindowExW(Some(root), None, CLASS, None) {
            SetWindowLongPtrW(host, GWLP_USERDATA, overlay.0 as isize);
        }
    }
}

pub fn set_visible(hwnd: HWND, visible: bool) {
    unsafe {
        if visible {
            // wry raises its own containers asynchronously, so restate the stack every time:
            // directly beneath the overlay if it exists, otherwise above everything (the shell).
            let overlay = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
            let after = if overlay != 0 {
                HWND(overlay as *mut _)
            } else {
                HWND_TOP
            };
            let _ = SetWindowPos(
                hwnd,
                Some(after),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
        let _ = ShowWindow(hwnd, if visible { SW_SHOWNA } else { SW_HIDE });
    }
}
