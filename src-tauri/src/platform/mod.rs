//! Native web-content engine abstraction.
//!
//! Web pages run in engine views that the browser core drives through [`BrowserEngine`].
//! Views report back through [`EngineEvent`]s delivered to a sink closure on the UI thread.
//! Web content views are deliberately *not* Tauri webviews: they get no Tauri IPC bridge,
//! no `window.__TAURI_INTERNALS__`, and no custom protocols (see docs/security.md).

use crate::adblock::AdBlockEngine;
use crate::config::Settings;
use crate::errors::AppResult;
use crate::permissions::PermissionKind;
use serde::Serialize;
use std::rc::Rc;
use std::sync::{Arc, RwLock};

#[cfg(windows)]
mod win;
#[cfg(windows)]
pub use self::win::WinEngine as PlatformEngine;
#[cfg(windows)]
pub use self::win::{restrict_ui_context_menu, set_overlay_window};

#[cfg(not(windows))]
mod unsupported;
#[cfg(not(windows))]
pub use self::unsupported::UnsupportedEngine as PlatformEngine;

pub type TabId = u64;
/// Engine-issued id for a pending popup or permission request awaiting a decision.
pub type RequestId = u64;

/// Physical-pixel rectangle relative to the window's client area.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DownloadState {
    InProgress,
    Completed,
    Interrupted,
    Cancelled,
}

impl DownloadState {
    pub fn as_str(self) -> &'static str {
        match self {
            DownloadState::InProgress => "inProgress",
            DownloadState::Completed => "completed",
            DownloadState::Interrupted => "interrupted",
            DownloadState::Cancelled => "cancelled",
        }
    }
}

/// Everything a content view reports. Delivered on the UI thread, possibly re-entrantly
/// from inside an engine call, so the receiver must queue rather than assume exclusivity.
#[derive(Debug, Clone)]
pub enum EngineEvent {
    ViewReady {
        tab: TabId,
    },
    ViewFailed {
        tab: TabId,
        error: String,
    },
    NavigationStarting {
        tab: TabId,
        url: String,
    },
    SourceChanged {
        tab: TabId,
        url: String,
    },
    TitleChanged {
        tab: TabId,
        title: String,
    },
    HistoryChanged {
        tab: TabId,
        can_go_back: bool,
        can_go_forward: bool,
    },
    NavigationCompleted {
        tab: TabId,
        success: bool,
        /// The URL the view actually shows afterwards (unchanged if the navigation did not commit).
        committed_url: String,
    },
    /// HTTPS-only mode tried https:// and failed; the user may choose to continue over http.
    /// A top-level load failed (DNS, timeout, certificate...). `reason` is a short code.
    LoadFailed {
        tab: TabId,
        url: String,
        reason: &'static str,
    },
    HttpsUpgradeFailed {
        tab: TabId,
        http_url: String,
    },
    FaviconChanged {
        tab: TabId,
        /// The document the icon belongs to (it arrives asynchronously, maybe after navigating on).
        page_url: String,
        data_url: Option<String>,
    },
    ZoomChanged {
        tab: TabId,
        zoom: f64,
    },
    RequestBlocked {
        tab: TabId,
    },
    NewWindowRequested {
        opener: TabId,
        request: RequestId,
        url: String,
        user_initiated: bool,
        background: bool,
    },
    /// "Open link in new tab" from the page context menu (opens in the background).
    OpenLink {
        opener: TabId,
        url: String,
        private: bool,
    },
    CloseRequested {
        tab: TabId,
    },
    FullscreenChanged {
        tab: TabId,
        fullscreen: bool,
    },
    PermissionRequested {
        tab: TabId,
        request: RequestId,
        origin: String,
        kind: PermissionKind,
    },
    ExternalProtocolRequested {
        tab: TabId,
        uri: String,
    },
    DownloadStarted {
        tab: TabId,
        id: i64,
        url: String,
        path: String,
        total: Option<i64>,
    },
    DownloadProgress {
        id: i64,
        received: i64,
        total: Option<i64>,
    },
    DownloadFinished {
        id: i64,
        state: DownloadState,
        received: i64,
        path: String,
    },
    FindResult {
        tab: TabId,
        matches: i32,
        active: i32,
    },
    Shortcut {
        tab: TabId,
        action: &'static str,
    },
    FocusLeft {
        tab: TabId,
    },
    /// The user clicked into or focused a view (used to pick the active pane in split view).
    ViewFocused {
        tab: TabId,
    },
    Crashed {
        tab: TabId,
    },
    /// The page has a password field (reported once per document).
    LoginFormFound {
        tab: TabId,
        origin: String,
    },
    /// A login form was submitted. Values come from the page and are unvalidated.
    LoginSubmitted {
        tab: TabId,
        origin: String,
        username: String,
        password: crate::passwords::Secret,
    },
    /// The submitted login form went away without a navigation (single-page apps): it worked.
    LoginSucceeded {
        tab: TabId,
    },
}

impl EngineEvent {
    /// The tab an event comes from (the opener for new-window events); `None` for download
    /// progress. The core routes by it, so a tab moved to another window keeps reporting there.
    pub fn tab(&self) -> Option<TabId> {
        use EngineEvent as E;
        Some(match self {
            E::NewWindowRequested { opener, .. } | E::OpenLink { opener, .. } => *opener,
            E::DownloadProgress { .. } | E::DownloadFinished { .. } => return None,
            E::ViewReady { tab }
            | E::ViewFailed { tab, .. }
            | E::NavigationStarting { tab, .. }
            | E::SourceChanged { tab, .. }
            | E::TitleChanged { tab, .. }
            | E::HistoryChanged { tab, .. }
            | E::NavigationCompleted { tab, .. }
            | E::LoadFailed { tab, .. }
            | E::HttpsUpgradeFailed { tab, .. }
            | E::FaviconChanged { tab, .. }
            | E::ZoomChanged { tab, .. }
            | E::RequestBlocked { tab }
            | E::CloseRequested { tab }
            | E::FullscreenChanged { tab, .. }
            | E::PermissionRequested { tab, .. }
            | E::ExternalProtocolRequested { tab, .. }
            | E::DownloadStarted { tab, .. }
            | E::FindResult { tab, .. }
            | E::Shortcut { tab, .. }
            | E::FocusLeft { tab }
            | E::ViewFocused { tab }
            | E::Crashed { tab }
            | E::LoginFormFound { tab, .. }
            | E::LoginSubmitted { tab, .. }
            | E::LoginSucceeded { tab } => *tab,
        })
    }
}

/// Browsing-data categories for [`BrowserEngine::clear_data`].
#[derive(Clone, Copy, Debug, Default)]
pub struct ClearDataKinds {
    pub cookies: bool,
    pub cache: bool,
    /// The engine's own record of visited pages (Veyscape's history is in SQLite).
    pub history: bool,
    pub downloads: bool,
    /// Unix ms; engine history and download records from then on go (`0` = all). Cookies and
    /// cache are always cleared entirely.
    pub since_ms: i64,
}

/// Engine-wide preferences applied to every browsing profile (normal and private).
#[derive(Clone, Debug, PartialEq)]
pub struct ProfilePrefs {
    pub tracking_prevention: crate::config::TrackingPrevention,
    pub theme: crate::config::Theme,
    pub download_dir: Option<String>,
}

impl ProfilePrefs {
    /// Preferences for one view: private tabs always get strict tracking prevention. They run
    /// in the InPrivate profile, so this never changes the setting for normal tabs.
    pub fn for_view(&self, private: bool) -> ProfilePrefs {
        let mut prefs = self.clone();
        if private {
            prefs.tracking_prevention = crate::config::TrackingPrevention::Strict;
        }
        prefs
    }
}

impl From<&Settings> for ProfilePrefs {
    fn from(s: &Settings) -> Self {
        ProfilePrefs {
            tracking_prevention: s.tracking_prevention,
            theme: s.theme,
            download_dir: s.download_dir.clone(),
        }
    }
}

/// Inputs a platform engine needs from the rest of the app.
pub struct EngineHooks {
    pub sink: Rc<dyn Fn(EngineEvent)>,
    pub filter: Arc<AdBlockEngine>,
    pub settings: Arc<RwLock<Settings>>,
}

/// Contract every platform adapter implements. All methods run on the UI thread and must not
/// block: view creation completes asynchronously with [`EngineEvent::ViewReady`].
pub trait BrowserEngine {
    /// Creates a content view. `popup` adopts a pending `NewWindowRequested` so `window.opener` works.
    fn create_view(&mut self, tab: TabId, private: bool, popup: Option<RequestId>)
        -> AppResult<()>;
    fn destroy_view(&mut self, tab: TabId);
    fn navigate(&mut self, tab: TabId, url: &str) -> AppResult<()>;
    fn reload(&self, tab: TabId) -> AppResult<()>;
    fn stop(&self, tab: TabId) -> AppResult<()>;
    fn go_back(&self, tab: TabId) -> AppResult<()>;
    fn go_forward(&self, tab: TabId) -> AppResult<()>;
    fn set_zoom(&self, tab: TabId, factor: f64) -> AppResult<()>;
    /// Places the content area (window client coordinates) and shows exactly the listed views at
    /// rects relative to it; every other view is hidden. Empty `panes` hides the content area.
    fn layout(&mut self, area: Rect, panes: &[(TabId, Rect)]);
    fn focus_view(&self, tab: TabId);
    fn find(&mut self, tab: TabId, text: &str, forward: bool) -> AppResult<()>;
    fn find_stop(&mut self, tab: TabId);
    fn print(&self, tab: TabId) -> AppResult<()>;
    fn resolve_permission(&mut self, request: RequestId, allow: bool);
    fn deny_popup(&mut self, request: RequestId);
    /// Lets one host load over plain http in this tab after an HTTPS-only failure, then loads it.
    fn allow_http(&mut self, tab: TabId, http_url: &str) -> AppResult<()>;
    fn cancel_download(&mut self, id: i64) -> AppResult<()>;
    fn resume_download(&mut self, id: i64) -> AppResult<()>;
    /// Releases the engine's handle to a download the core no longer tracks (e.g. an interrupted
    /// one the user removed); it can no longer be resumed or cancelled afterwards.
    fn forget_download(&mut self, id: i64);
    fn clear_data(&mut self, kinds: ClearDataKinds, done: Box<dyn FnOnce(AppResult<()>)>);
    fn apply_prefs(&mut self, prefs: ProfilePrefs);
    fn notify_window_moved(&self);
    /// True when keyboard focus is not inside this window's webviews: on the bare frame, nowhere,
    /// or still in another (closing) window.
    fn focus_needs_restoring(&self) -> bool;
    /// Human-readable engine name and version for the About page.
    fn engine_version(&self) -> String;
    /// Types a saved login into the login form on the tab's current page if that page is still
    /// on `origin`. `done` runs exactly once, with whether a field was filled.
    fn fill_login(
        &self,
        tab: TabId,
        origin: &str,
        username: &str,
        password: &str,
        done: Box<dyn FnOnce(AppResult<bool>)>,
    );
    /// Takes a tab's live view out of this engine (it is hidden, not closed) so another window's
    /// engine can [`attach_view`](Self::attach_view) it. `None` if the view does not exist or is
    /// still being created (that creation is abandoned).
    fn detach_view(&mut self, tab: TabId) -> Option<Box<dyn std::any::Any>>;
    /// Adopts a view detached from another window's engine; it shows on the next layout.
    fn attach_view(&mut self, tab: TabId, view: Box<dyn std::any::Any>) -> AppResult<()>;
}
