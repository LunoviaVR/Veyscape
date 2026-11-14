//! Placeholder adapter for platforms without an implementation yet (macOS: WKWebView,
//! Linux: WebKitGTK; see docs/platform-support.md). Every operation reports `Unsupported`
//! so the UI shows an honest error instead of pretending to browse.

use super::*;
use crate::errors::AppError;

pub struct UnsupportedEngine;

impl UnsupportedEngine {
    pub fn new(_window: &tauri::Window, _hooks: EngineHooks) -> AppResult<Self> {
        Ok(UnsupportedEngine)
    }
}

const MSG: &str = "web content views are only implemented for Windows (WebView2) so far";

impl BrowserEngine for UnsupportedEngine {
    fn create_view(&mut self, _: TabId, _: bool, _: Option<RequestId>) -> AppResult<()> {
        Err(crate::errors::AppError::Unsupported(MSG))
    }
    fn destroy_view(&mut self, _: TabId) {}
    fn navigate(&mut self, _: TabId, _: &str) -> AppResult<()> {
        Err(crate::errors::AppError::Unsupported(MSG))
    }
    fn reload(&self, _: TabId) -> AppResult<()> {
        Ok(())
    }
    fn stop(&self, _: TabId) -> AppResult<()> {
        Ok(())
    }
    fn go_back(&self, _: TabId) -> AppResult<()> {
        Ok(())
    }
    fn go_forward(&self, _: TabId) -> AppResult<()> {
        Ok(())
    }
    fn set_zoom(&self, _: TabId, _: f64) -> AppResult<()> {
        Ok(())
    }
    fn layout(&mut self, _: Rect, _: &[(TabId, Rect)]) {}
    fn focus_view(&self, _: TabId) {}
    fn find(&mut self, _: TabId, _: &str, _: bool) -> AppResult<()> {
        Err(crate::errors::AppError::Unsupported(MSG))
    }
    fn find_stop(&mut self, _: TabId) {}
    fn print(&self, _: TabId) -> AppResult<()> {
        Err(crate::errors::AppError::Unsupported(MSG))
    }
    fn resolve_permission(&mut self, _: RequestId, _: bool) {}
    fn deny_popup(&mut self, _: RequestId) {}
    fn allow_http(&mut self, _: TabId, _: &str) -> AppResult<()> {
        Ok(())
    }
    fn cancel_download(&mut self, _: i64) -> AppResult<()> {
        Ok(())
    }
    fn resume_download(&mut self, _: i64) -> AppResult<()> {
        Ok(())
    }
    fn forget_download(&mut self, _: i64) {}
    fn clear_data(&mut self, _: ClearDataKinds, done: Box<dyn FnOnce(AppResult<()>)>) {
        done(Err(crate::errors::AppError::Unsupported(MSG)))
    }
    fn apply_prefs(&mut self, _: ProfilePrefs) {}
    fn notify_window_moved(&self) {}
    fn focus_needs_restoring(&self) -> bool {
        true
    }
    fn engine_version(&self) -> String {
        "unsupported platform".into()
    }
    fn fill_login(
        &self,
        _: TabId,
        _: &str,
        _: &str,
        _: &str,
        done: Box<dyn FnOnce(AppResult<bool>)>,
    ) {
        done(Err(AppError::Unsupported("password filling")))
    }
    fn detach_view(&mut self, _: TabId) -> Option<Box<dyn std::any::Any>> {
        None
    }
    fn attach_view(&mut self, _: TabId, _: Box<dyn std::any::Any>) -> AppResult<()> {
        Err(AppError::Unsupported("moving tabs between windows"))
    }
}
