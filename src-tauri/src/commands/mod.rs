//! Tauri command handlers: thin, validated entry points from the trusted shell UI.
//!
//! Access control is layered:
//! 1. The capability ACL (`capabilities/shell.json`) allows these commands only for the `shell-*`
//!    and `overlay-*` webviews of browser windows, from the bundled app origin.
//! 2. `run()` wraps the invoke handler and rejects any other caller label (defense in depth).
//! 3. Web content tabs are raw WebView2 controllers with no Tauri IPC at all.
//!
//! Commands that touch browser state are synchronous: Tauri runs them on the UI thread, which
//! is where the browser core lives.

pub mod browser;
pub mod library;
pub mod passwords;
pub mod settings;

use crate::errors::{AppError, AppResult};

/// Runs `start` on the UI thread and waits, off it, for the one answer `start` passes to its
/// callback (or the error `start` returns). No answer within `timeout` is `timed_out`.
async fn answered_on_ui_thread<T: Send + 'static>(
    app: &tauri::AppHandle,
    timeout: std::time::Duration,
    timed_out: &'static str,
    start: impl FnOnce(Box<dyn FnOnce(AppResult<T>)>) -> AppResult<()> + Send + 'static,
) -> AppResult<T> {
    let (tx, rx) = std::sync::mpsc::channel::<AppResult<T>>();
    app.run_on_main_thread(move || {
        let tx2 = tx.clone();
        if let Err(e) = start(Box::new(move |r| {
            let _ = tx2.send(r);
        })) {
            let _ = tx.send(Err(e));
        }
    })?;
    tauri::async_runtime::spawn_blocking(move || {
        rx.recv_timeout(timeout)
            .map_err(|_| AppError::Engine(timed_out.into()))?
    })
    .await
    .map_err(|e| AppError::Engine(e.to_string()))?
}

/// A browser window's shell or overlay webview (`shell-<n>`, `overlay-<n>`).
pub fn is_trusted_caller(webview_label: &str) -> bool {
    (webview_label.starts_with("shell-") || webview_label.starts_with("overlay-"))
        && crate::browser::window_id(webview_label).is_some()
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_shell_and_overlay_are_trusted() {
        assert!(super::is_trusted_caller("shell-1"));
        assert!(super::is_trusted_caller("overlay-12"));
        assert!(!super::is_trusted_caller("shell"));
        assert!(!super::is_trusted_caller("window-1"));
        assert!(!super::is_trusted_caller("shell-1x"));
        assert!(!super::is_trusted_caller("shell-+1"));
        assert!(!super::is_trusted_caller("tab-1"));
    }
}
