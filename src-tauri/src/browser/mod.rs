//! Browser core: owns every browser window's tabs, drives each window's platform engine, reacts
//! to engine events, and publishes state to the shells. Lives on the UI thread only
//! (`thread_local`), so no locks are needed.
//!
//! [`Browser`] holds what all windows share (downloads, the saved session); each [`Window`] holds
//! one window's tabs, groups, split view and engine.
//!
//! Re-entrancy: engine calls may raise events synchronously. Events are queued and processed
//! after the current `with_browser` call releases its borrow (see `dispatch`).

mod logins;
pub mod session;
pub mod shortcuts;
pub mod split;
pub mod tabs;
pub mod url;

use crate::config::{Settings, StartupBehavior};
use crate::downloads::DownloadItem;
use crate::errors::{AppError, AppResult};
use crate::permissions::{self, Decision, PermissionKind};
use crate::platform::{
    BrowserEngine, ClearDataKinds, DownloadState, EngineEvent, ProfilePrefs, Rect, RequestId, TabId,
};
use crate::state::Services;
use serde::Serialize;
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tabs::{Tab, TabError, TabManager};
use tauri::{AppHandle, Emitter, EventTarget, Manager};
use url::{resolve_input, NEW_TAB_URL};

/// Identifies one browser window; its Tauri labels are `window-<id>`, `shell-<id>`, `overlay-<id>`.
pub type WindowId = u32;

pub fn window_label(id: WindowId) -> String {
    format!("window-{id}")
}

pub fn shell_label(id: WindowId) -> String {
    format!("shell-{id}")
}

pub fn overlay_label(id: WindowId) -> String {
    format!("overlay-{id}")
}

/// The window a label belongs to (`window-3`, `shell-3` and `overlay-3` all give 3).
pub fn window_id(label: &str) -> Option<WindowId> {
    let (kind, n) = label.split_once('-')?;
    let digits = !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit());
    (matches!(kind, "window" | "shell" | "overlay") && digits)
        .then(|| n.parse().ok())
        .flatten()
}

pub fn next_window_id() -> WindowId {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed) + 1
}

thread_local! {
    /// Tabs on their way to a new window that is still being created (UI thread only).
    static TEARING_OFF: RefCell<std::collections::HashSet<TabId>> = RefCell::default();
    static BROWSER: RefCell<Option<Browser>> = const { RefCell::new(None) };
    static QUEUE: RefCell<VecDeque<(WindowId, EngineEvent)>> = const { RefCell::new(VecDeque::new()) };
}

/// An icon reported for `page_url` still belongs to what the tab shows: the same site, and not
/// an internal page shown over it.
fn icon_is_current(t: &Tab, page_url: &str) -> bool {
    t.internal.is_none() && url::origin_of(&t.url) == url::origin_of(page_url)
}

/// Notes that `tab` will move into a window being created (`WindowSeed::Adopt`). A drop of the
/// same tab before that window is ready cancels the move (`Browser::drop_tab`).
pub fn begin_tear_off(tab: TabId) {
    TEARING_OFF.with(|s| s.borrow_mut().insert(tab));
}

/// Takes `tab` off the pending tear-offs; false if it was not pending (cancelled meanwhile).
fn end_tear_off(tab: TabId) -> bool {
    TEARING_OFF.with(|s| s.borrow_mut().remove(&tab))
}

/// Runs `f` with exclusive access to the browser, then processes queued engine events.
/// Must be called on the UI thread (sync Tauri commands and window events run there).
pub fn with_browser<T>(f: impl FnOnce(&mut Browser) -> T) -> AppResult<T> {
    let result = BROWSER.with(|cell| {
        let mut guard = cell
            .try_borrow_mut()
            .map_err(|_| AppError::Engine("browser busy".into()))?;
        let browser = guard
            .as_mut()
            .ok_or_else(|| AppError::Engine("browser not initialized".into()))?;
        Ok(f(browser))
    });
    drain();
    result
}

/// Runs `f` on one window's state.
pub fn with_window<T>(id: WindowId, f: impl FnOnce(&mut Window) -> T) -> AppResult<T> {
    with_browser(|b| {
        b.window_mut(id)
            .map(f)
            .ok_or_else(|| AppError::NotFound("window".into()))
    })?
}

/// Runs `f` on the window whose shell or overlay made the current command call.
pub fn with_caller<T>(webview: &tauri::Webview, f: impl FnOnce(&mut Window) -> T) -> AppResult<T> {
    let id = window_id(webview.label())
        .ok_or_else(|| AppError::InvalidInput("caller is not a browser window".into()))?;
    with_window(id, f)
}

/// Engine event sink for `window`: queue, then process unless nested inside a browser call.
pub fn dispatch(window: WindowId, event: EngineEvent) {
    QUEUE.with(|q| q.borrow_mut().push_back((window, event)));
    drain();
}

fn drain() {
    loop {
        let processed = BROWSER.with(|cell| {
            let Ok(mut guard) = cell.try_borrow_mut() else {
                return false;
            };
            let Some(browser) = guard.as_mut() else {
                return false;
            };
            match QUEUE.with(|q| q.borrow_mut().pop_front()) {
                Some((window, event)) => {
                    browser.handle_event(window, event);
                    true
                }
                None => {
                    browser.flush();
                    false
                }
            }
        });
        if !processed {
            break;
        }
    }
}

pub fn install(browser: Browser) {
    BROWSER.with(|cell| *cell.borrow_mut() = Some(browser));
    drain();
}

#[derive(Serialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Prompt {
    #[serde(rename_all = "camelCase")]
    Permission {
        id: u64,
        tab_id: TabId,
        origin: String,
        kind: PermissionKind,
        #[serde(skip)]
        request: RequestId,
    },
    #[serde(rename_all = "camelCase")]
    ExternalProtocol { id: u64, tab_id: TabId, uri: String },
    #[serde(rename_all = "camelCase")]
    PopupBlocked {
        id: u64,
        tab_id: TabId,
        origin: String,
        url: String,
    },
    /// "Save password?" after a login. Answered with allow = save, remember = never for this site.
    #[serde(rename_all = "camelCase")]
    SavePassword {
        id: u64,
        tab_id: TabId,
        origin: String,
        username: String,
        /// Replaces the saved password of this username instead of adding a login.
        update: bool,
        #[serde(skip)]
        password: crate::passwords::Secret,
    },
}

impl Prompt {
    fn id(&self) -> u64 {
        match self {
            Prompt::Permission { id, .. }
            | Prompt::ExternalProtocol { id, .. }
            | Prompt::PopupBlocked { id, .. }
            | Prompt::SavePassword { id, .. } => *id,
        }
    }

    fn tab_id(&self) -> TabId {
        match self {
            Prompt::Permission { tab_id, .. }
            | Prompt::ExternalProtocol { tab_id, .. }
            | Prompt::PopupBlocked { tab_id, .. }
            | Prompt::SavePassword { tab_id, .. } => *tab_id,
        }
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot<'a> {
    /// A private window (only private tabs).
    pub private_window: bool,
    pub tabs: &'a [Tab],
    pub active_tab_id: Option<TabId>,
    pub can_reopen_closed: bool,
    pub prompts: &'a [Prompt],
    pub fullscreen: bool,
    pub groups: &'a [tabs::TabGroup],
    pub split: Option<SplitState<'a>>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SplitState<'a> {
    pub tabs: &'a [TabId],
    pub layout: split::SplitLayout,
    /// True while the active tab is one of the panes.
    pub shown: bool,
    /// Pane rectangles in CSS px relative to the content area (empty when not shown).
    pub panes: Vec<Pane>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Pane {
    pub tab_id: TabId,
    pub rect: split::RectF,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitialState<'a> {
    pub state: Snapshot<'a>,
    pub favicons: HashMap<TabId, String>,
    pub downloads: &'a [DownloadItem],
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Task {
    Emit,
    SaveSession,
    CloseWindow(WindowId),
}

/// What a new window starts with.
pub enum WindowSeed {
    /// First window at launch without a saved session: the homepage or a new tab, per settings.
    Startup,
    /// A window from the saved session.
    Restore(session::WindowSession),
    /// A window the user opened (Ctrl+N, launching Veyscape again): one new tab.
    New,
    /// A private window: every tab in it is private, and it is never saved in the session.
    Private,
    /// A tab moved out of window `from` (dragged, or "Move to new window") with its page as it
    /// is. `at`: where it was dropped (screen, logical px); `None` lets Windows place the window.
    Adopt {
        from: WindowId,
        tab: TabId,
        at: Option<(f64, f64)>,
    },
}

/// A tab on its way to another window, with its engine view (if it had a ready one).
pub(super) struct DetachedTab {
    tab: Tab,
    view: Option<Box<dyn std::any::Any>>,
    /// Its current navigation turned into a download (see `Window::download_navigations`).
    download_navigation: bool,
}

/// Windows to open at launch: the saved session's windows, or one start window.
pub fn startup_seeds(services: &Services) -> Vec<WindowSeed> {
    let restored = match services.settings().startup {
        StartupBehavior::RestoreSession => session::load(&services.db),
        _ => None,
    };
    let seeds: Vec<WindowSeed> = restored
        .map(|s| s.windows.into_iter().map(WindowSeed::Restore).collect())
        .unwrap_or_default();
    if seeds.is_empty() {
        vec![WindowSeed::Startup]
    } else {
        seeds
    }
}

/// State shared by all windows. History, bookmarks and settings live in the shared database,
/// so every window reads the same data; this publishes changes to all of them.
pub struct Browser {
    app: AppHandle,
    services: Arc<Services>,
    windows: Vec<Window>,
    downloads: Vec<DownloadItem>,
    scheduled: Vec<Task>,
    /// Set while quitting: windows torn down then must not rewrite the saved session.
    exiting: bool,
    /// The last window with tabs to save closed while private windows stay open: its saved
    /// session is kept until a window again has a tab to save, so it comes back next time.
    keep_saved_session: bool,
    /// A private tab is open somewhere (the private session is alive).
    private_session: bool,
    /// The tab being dragged in a tab strip and when the drag began (`start_tab_drag`).
    dragged_tab: Option<(TabId, Instant)>,
}

/// One browser window: its tabs, groups, split view, prompts and engine.
pub struct Window {
    id: WindowId,
    /// A private window: every tab is private and the window is never saved in the session.
    private: bool,
    app: AppHandle,
    services: Arc<Services>,
    engine: Box<dyn BrowserEngine>,
    tabs: TabManager,
    prompts: Vec<Prompt>,
    chrome_height: f64,
    page_fullscreen: bool,
    window_fullscreen: bool,
    next_prompt: u64,
    state_dirty: bool,
    last_emit: Instant,
    session_dirty: bool,
    /// A visit or title was written to history; every window's History page should reload.
    history_dirty: bool,
    /// The last tab was closed; the window closes once the current call returns.
    close_requested: bool,
    /// Restored maximized: maximize when the window is first shown.
    pending_maximize: bool,
    /// Tabs whose current navigation turned into a download (consumed on NavigationCompleted).
    download_navigations: std::collections::HashSet<TabId>,
    /// Submitted logins not yet offered for saving (see `logins.rs`).
    pending_logins: HashMap<TabId, logins::PendingLogin>,
    split: Option<split::Split>,
    /// Pane geometry of the split view currently shown (CSS px, relative to the content area).
    panes: Vec<(TabId, split::RectF)>,
}

impl Browser {
    pub fn new(app: AppHandle, services: Arc<Services>) -> Self {
        let downloads = services
            .db
            .downloads_list(200)
            .unwrap_or_default()
            .into_iter()
            .map(DownloadItem::from_record)
            .collect();
        Browser {
            app,
            services,
            windows: Vec::new(),
            downloads,
            scheduled: Vec::new(),
            exiting: false,
            keep_saved_session: false,
            private_session: false,
            dragged_tab: None,
        }
    }

    /// A tab strip started dragging one of its window's tabs. Drops (`drop_tab`) act only on a
    /// tab registered here, so a drag started by a web page cannot move tabs.
    pub fn start_tab_drag(&mut self, window: WindowId, tab: TabId) -> AppResult<()> {
        if !self
            .window(window)
            .is_some_and(|w| w.tabs.get(tab).is_some())
        {
            return Err(AppError::NotFound("tab".into()));
        }
        self.dragged_tab = Some((tab, Instant::now()));
        Ok(())
    }

    /// The dragged tab was dropped on window `to`'s tab strip before position `index` (joining
    /// `group` if dropped onto one). A tab from another window moves here with its page as it is;
    /// a window left without tabs closes.
    pub fn drop_tab(&mut self, to: WindowId, index: usize, group: Option<u64>) -> AppResult<()> {
        let (tab, started) = self
            .dragged_tab
            .take()
            .ok_or_else(|| AppError::NotFound("dragged tab".into()))?;
        if started.elapsed() > Duration::from_secs(60) {
            return Err(AppError::InvalidInput("the drag ended long ago".into()));
        }
        // Dropped before the window it was being torn off into got ready: it stays here instead.
        end_tear_off(tab);
        let (from, private_tab) = self
            .windows
            .iter()
            .find_map(|w| w.tabs.get(tab).map(|t| (w.id, t.private)))
            .ok_or_else(|| AppError::NotFound("tab".into()))?;
        if from != to {
            let target = self
                .window(to)
                .ok_or_else(|| AppError::NotFound("window".into()))?;
            if target.private && !private_tab {
                return Err(AppError::InvalidInput(
                    "only private tabs can go into a private window".into(),
                ));
            }
            let moved = self
                .window_mut(from)
                .and_then(|w| w.detach_tab(tab))
                .ok_or_else(|| AppError::NotFound("tab".into()))?;
            if let Some(w) = self.window_mut(to) {
                w.adopt_tab(moved);
            }
            // Like a click: the window the tab was dropped into comes to the front.
            if let Some(win) = self.app.get_window(&window_label(to)) {
                let _ = win.set_focus();
            }
        }
        if let Some(w) = self.window_mut(to) {
            w.move_tab(tab, index, group);
        }
        Ok(())
    }

    /// Adds a window once its engine exists (its shell webview's environment is ready).
    pub fn attach_window(
        &mut self,
        id: WindowId,
        mut engine: Box<dyn BrowserEngine>,
        seed: WindowSeed,
    ) {
        let settings = self.services.settings();
        engine.apply_prefs(ProfilePrefs::from(&settings));
        let private = match &seed {
            WindowSeed::Private => true,
            // A private tab torn off any window opens a private window: a normal one holding only
            // private tabs would be saved empty and wipe the session.
            WindowSeed::Adopt { from, tab, .. } => {
                self.window(*from).is_some_and(|w| w.private)
                    || self.tab(*tab).is_some_and(|t| t.private)
            }
            _ => false,
        };
        if private {
            if let Some(w) = self.app.get_window(&window_label(id)) {
                let _ = w.set_title("Veyscape (private)");
            }
        }
        let mut window = Window {
            id,
            private,
            app: self.app.clone(),
            services: self.services.clone(),
            engine,
            tabs: TabManager::for_window(id),
            prompts: Vec::new(),
            chrome_height: 0.0,
            page_fullscreen: false,
            window_fullscreen: false,
            next_prompt: 0,
            state_dirty: true,
            last_emit: Instant::now() - Duration::from_secs(1),
            session_dirty: false,
            history_dirty: false,
            close_requested: false,
            pending_maximize: false,
            download_navigations: Default::default(),
            pending_logins: HashMap::new(),
            split: None,
            panes: Vec::new(),
        };
        let moved = match &seed {
            WindowSeed::Adopt { from, tab, .. } => {
                if !end_tear_off(*tab) {
                    // The tab was dropped back into a tab strip before this window was ready.
                    window.close_requested = true;
                    self.windows.push(window);
                    return;
                }
                self.window_mut(*from).and_then(|w| w.detach_tab(*tab))
            }
            _ => None,
        };
        match moved {
            Some(moved) => window.adopt_tab(moved),
            // Also when the dragged tab was closed before the window got ready.
            None => window.start(seed, &settings),
        }
        self.windows.push(window);
    }

    pub fn window(&self, id: WindowId) -> Option<&Window> {
        self.windows.iter().find(|w| w.id == id)
    }

    pub fn window_mut(&mut self, id: WindowId) -> Option<&mut Window> {
        self.windows.iter_mut().find(|w| w.id == id)
    }

    /// Finds a tab in any window.
    pub fn tab(&self, id: TabId) -> Option<&Tab> {
        self.windows.iter().find_map(|w| w.tabs.get(id))
    }

    pub fn initial_state(&self, window: WindowId) -> Option<InitialState<'_>> {
        let w = self.window(window)?;
        Some(InitialState {
            state: w.snapshot(),
            favicons: w.favicons(),
            downloads: &self.downloads,
        })
    }

    // ---------- publication & scheduling ----------

    /// Publishes every window's pending state (≤ ~30 snapshots per second each), then schedules
    /// session saves and window closes.
    fn flush(&mut self) {
        let mut emit_later = None;
        let mut session = false;
        let mut history = false;
        let mut closing = Vec::new();
        for w in &mut self.windows {
            if let Some(delay) = w.flush() {
                emit_later = Some(delay);
            }
            session |= w.session_dirty;
            history |= std::mem::take(&mut w.history_dirty);
            if w.close_requested {
                closing.push(w.id);
            }
        }
        if let Some(delay) = emit_later {
            self.schedule(Task::Emit, delay);
        }
        if session {
            self.schedule(Task::SaveSession, Duration::from_secs(2));
        }
        if history {
            self.broadcast("history:changed", ());
        }
        for id in closing {
            self.request_close(id);
        }
        let private_open = self
            .windows
            .iter()
            .any(|w| w.tabs.tabs().iter().any(|t| t.private));
        if self.private_session && !private_open {
            self.end_private_session();
        }
        self.private_session = private_open;
    }

    /// The last private tab closed. WebView2 discards the private cookies, cache and site data
    /// itself; drop what Veyscape kept for the private session too.
    fn end_private_session(&mut self) {
        self.services.adblock.end_private_session();
        if crate::downloads::forget_private(&mut self.downloads) {
            self.publish_downloads();
        }
    }

    /// Sends an event to the UI of every window (shells and overlays).
    fn broadcast<S: Serialize + Clone>(&self, event: &str, payload: S) {
        if let Err(e) = self.app.emit(event, payload) {
            log::warn!("emit {event} failed: {e}");
        }
    }

    // Tasks run later from the event loop, outside any IPC callback.
    fn schedule(&mut self, task: Task, delay: Duration) {
        if self.scheduled.contains(&task) {
            return;
        }
        self.scheduled.push(task);
        let app = self.app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            let _ = app.run_on_main_thread(move || {
                let _ = with_browser(|b| b.run_task(task));
            });
        });
    }

    fn run_task(&mut self, task: Task) {
        self.scheduled.retain(|t| *t != task);
        match task {
            Task::Emit => {} // flush() runs after every with_browser call
            Task::SaveSession => self.save_session(),
            Task::CloseWindow(id) => self.close_window(id),
        }
    }

    pub fn save_session(&mut self) {
        for w in &mut self.windows {
            w.session_dirty = false;
        }
        let windows: Vec<_> = self
            .windows
            .iter()
            // Private windows are never restored.
            .filter(|w| !w.private)
            .map(|w| session::capture(&w.tabs, &w.session_extras()))
            .collect();
        if session::skip_save(&windows, &mut self.keep_saved_session) {
            return;
        }
        if let Err(e) = session::save(&self.services.db, windows) {
            log::error!("saving session failed: {e}");
        }
    }

    /// Closes a window (caption button, Alt+F4, its last tab) once the current call returns,
    /// so a window is never destroyed inside one of its own callbacks.
    pub fn request_close(&mut self, id: WindowId) {
        self.schedule(Task::CloseWindow(id), Duration::ZERO);
    }

    /// Closing the last window quits and keeps it in the saved session; closing any other
    /// window forgets it, as in other browsers.
    fn close_window(&mut self, id: WindowId) {
        // Already quitting: a second quit would exit before "clear cookies on exit" finished.
        if self.exiting {
            return;
        }
        let Some(index) = self.windows.iter().position(|w| w.id == id) else {
            return;
        };
        if self.windows.len() == 1 {
            return self.quit();
        }
        // The last window with tabs to save (only private tabs stay open) is kept in the session
        // as it is now, as in Chrome.
        let saves: Vec<bool> = self
            .windows
            .iter()
            .map(|w| !w.private && w.tabs.tabs().iter().any(|t| !t.private))
            .collect();
        if session::is_last_saved_window(index, &saves) {
            self.save_session();
            self.keep_saved_session = true;
        }
        // Pending permission requests of its tabs are answered, not left waiting in the engine.
        let w = &mut self.windows[index];
        let tabs: Vec<TabId> = w.tabs.tabs().iter().map(|t| t.id).collect();
        for tab in tabs {
            w.deny_prompts_for(tab);
        }
        // Dropping the window closes its engine's views.
        drop(self.windows.remove(index));
        self.save_session();
        if let Some(w) = self.app.get_window(&window_label(id)) {
            let _ = w.destroy();
        }
    }

    /// Saves the session with every open window, then exits.
    pub fn quit(&mut self) {
        self.prepare_exit();
        self.app.exit(0);
    }

    /// Final session save; windows torn down after this must not overwrite it.
    pub fn prepare_exit(&mut self) {
        if !self.exiting {
            self.save_session();
            self.exiting = true;
        }
    }

    // ---------- downloads (shared by all windows) ----------

    pub fn downloads(&self) -> &[DownloadItem] {
        &self.downloads
    }

    fn download_mut(&mut self, id: i64) -> Option<&mut DownloadItem> {
        self.downloads.iter_mut().find(|d| d.id == id)
    }

    fn publish_downloads(&self) {
        self.broadcast("downloads:state", &self.downloads);
    }

    fn persist_download(&self, id: i64) {
        if let Some(d) = self.downloads.iter().find(|d| d.id == id && !d.private) {
            if let Err(e) = self.services.db.download_upsert(&d.record()) {
                log::error!("saving download failed: {e}");
            }
        }
    }

    /// Every window's engine sees all downloads of the session (one shared map), so any will do;
    /// the engine reports downloads it does not know (from earlier sessions).
    fn download_engine(&mut self) -> AppResult<&mut Box<dyn BrowserEngine>> {
        self.windows
            .first_mut()
            .map(|w| &mut w.engine)
            .ok_or_else(|| AppError::Engine("no browser window is open".into()))
    }

    pub fn cancel_download(&mut self, id: i64) -> AppResult<()> {
        self.download_engine()?.cancel_download(id)
    }

    pub fn resume_download(&mut self, id: i64) -> AppResult<()> {
        self.download_engine()?.resume_download(id)?;
        if let Some(d) = self.download_mut(id) {
            d.state = DownloadState::InProgress.as_str().into();
        }
        self.publish_downloads();
        Ok(())
    }

    pub fn download_path(&self, id: i64) -> AppResult<String> {
        self.downloads
            .iter()
            .find(|d| d.id == id)
            .map(|d| d.path.clone())
            .ok_or_else(|| AppError::NotFound("download".into()))
    }

    pub fn remove_download(&mut self, id: i64) -> AppResult<()> {
        if self
            .downloads
            .iter()
            .any(|d| d.id == id && d.state == "inProgress")
        {
            return Err(AppError::InvalidInput(
                "cancel the download before removing it".into(),
            ));
        }
        self.downloads.retain(|d| d.id != id);
        if let Ok(engine) = self.download_engine() {
            engine.forget_download(id);
        }
        self.services.db.download_remove(id)?;
        self.publish_downloads();
        Ok(())
    }

    /// Removes finished downloads started at or after `since` (Unix ms, 0 = all) from the list.
    pub fn clear_downloads(&mut self, since: i64) -> AppResult<()> {
        let cleared = |d: &DownloadItem| d.state != "inProgress" && d.started_at >= since;
        let finished: Vec<i64> = self
            .downloads
            .iter()
            .filter(|d| cleared(d))
            .map(|d| d.id)
            .collect();
        if let Ok(engine) = self.download_engine() {
            for id in finished {
                engine.forget_download(id);
            }
        }
        self.downloads.retain(|d| !cleared(d));
        self.services.db.downloads_clear_finished(since)?;
        self.publish_downloads();
        Ok(())
    }

    // ---------- settings & profile (shared by all windows) ----------

    pub fn apply_settings(&mut self, settings: &Settings) {
        for w in &mut self.windows {
            w.engine.apply_prefs(ProfilePrefs::from(settings));
        }
    }

    /// All windows browse in one profile, so any window's engine can clear it.
    pub fn clear_data(&mut self, kinds: ClearDataKinds, done: Box<dyn FnOnce(AppResult<()>)>) {
        match self.windows.first_mut() {
            Some(w) => w.engine.clear_data(kinds, done),
            None => done(Err(AppError::Engine("no browser window is open".into()))),
        }
    }

    pub fn engine_version(&self) -> String {
        self.windows
            .first()
            .map(|w| w.engine.engine_version())
            .unwrap_or_default()
    }

    /// Re-reads bookmark flags in every window (after any bookmark change).
    pub fn refresh_bookmarks(&mut self) {
        for w in &mut self.windows {
            w.refresh_bookmarks();
        }
    }

    // ---------- engine events ----------

    fn handle_event(&mut self, window: WindowId, event: EngineEvent) {
        use EngineEvent as E;
        // A tab moved to another window still reports through the engine it was created in.
        let window = event
            .tab()
            .and_then(|t| self.windows.iter().find(|w| w.tabs.get(t).is_some()))
            .map_or(window, |w| w.id);
        match event {
            E::DownloadStarted {
                tab,
                id,
                url,
                path,
                total,
            } => {
                let mut private = true;
                if let Some(w) = self.window_mut(window) {
                    private = w.tabs.get(tab).is_none_or(|t| t.private);
                    if w.tabs.get(tab).is_some_and(|t| t.url == url) {
                        w.download_navigations.insert(tab);
                    }
                }
                self.downloads
                    .insert(0, DownloadItem::new(id, url, path, total, private));
                self.persist_download(id);
                self.publish_downloads();
                if let Some(w) = self.window(window) {
                    w.emit_shell("downloads:started", id);
                }
            }
            E::DownloadProgress {
                id,
                received,
                total,
            } => {
                if let Some(d) = self.download_mut(id) {
                    d.received = received;
                    d.total = total.or(d.total);
                }
                self.publish_downloads();
            }
            E::DownloadFinished {
                id,
                state,
                received,
                path,
            } => {
                if let Some(d) = self.download_mut(id) {
                    d.state = state.as_str().into();
                    d.received = received;
                    if !path.is_empty() {
                        d.set_path(path);
                    }
                }
                self.persist_download(id);
                // A private download that outlived the private session leaves the list now.
                if !self.private_session {
                    crate::downloads::forget_private(&mut self.downloads);
                }
                self.publish_downloads();
            }
            // Events from a window closed meanwhile are dropped with it, but a request still
            // waiting in the engine (shared by all windows) must be answered.
            other => match self.window_mut(window) {
                Some(w) => w.handle_event(other),
                None => {
                    if let Some(w) = self.windows.first_mut() {
                        match other {
                            E::PermissionRequested { request, .. } => {
                                w.engine.resolve_permission(request, false)
                            }
                            E::NewWindowRequested { request, .. } => w.engine.deny_popup(request),
                            _ => {}
                        }
                    }
                }
            },
        }
    }
}

impl Window {
    pub fn id(&self) -> WindowId {
        self.id
    }

    fn start(&mut self, seed: WindowSeed, settings: &Settings) {
        match seed {
            WindowSeed::Restore(s) if !s.tabs.is_empty() => {
                let group_ids: HashMap<u64, u64> = s
                    .groups
                    .iter()
                    .map(|g| (g.id, self.tabs.restore_group(g)))
                    .collect();
                let mut ids = Vec::with_capacity(s.tabs.len());
                for saved in &s.tabs {
                    let id = self.tabs.allocate_id();
                    ids.push(id);
                    let mut tab = Tab::new(id, &saved.url, false);
                    tab.pinned = saved.pinned;
                    tab.group_id = saved.group.and_then(|g| group_ids.get(&g).copied());
                    if !saved.title.is_empty() {
                        tab.title = saved.title.clone();
                    }
                    self.tabs.insert(tab, None);
                }
                self.split = s.split.as_ref().and_then(|saved| {
                    let tabs: Option<Vec<TabId>> =
                        saved.tabs.iter().map(|&i| ids.get(i).copied()).collect();
                    tabs.map(|tabs| split::Split {
                        tabs,
                        layout: saved.layout,
                    })
                });
                self.pending_maximize = s.maximized;
                let active = self.tabs.nth(s.active.min(self.tabs.len() - 1));
                // Only the active tab (or its split panes) loads now; the rest load when selected.
                if let Some(id) = active {
                    self.activate(id);
                }
            }
            WindowSeed::New | WindowSeed::Private | WindowSeed::Adopt { .. } => {
                self.create_tab(None, false, true, None);
            }
            _ => {
                let url = match settings.startup {
                    StartupBehavior::Homepage if !settings.homepage.is_empty() => {
                        settings.homepage.clone()
                    }
                    _ => NEW_TAB_URL.to_string(),
                };
                self.create_tab(Some(url), false, true, None);
            }
        }
    }

    // ---------- state publication ----------

    fn mark_dirty(&mut self) {
        self.state_dirty = true;
    }

    fn mark_session_dirty(&mut self) {
        self.session_dirty = true;
        self.mark_dirty();
    }

    pub fn snapshot(&self) -> Snapshot<'_> {
        Snapshot {
            private_window: self.private,
            tabs: self.tabs.tabs(),
            active_tab_id: self.tabs.active_id(),
            can_reopen_closed: self.tabs.can_reopen(),
            prompts: &self.prompts,
            fullscreen: self.page_fullscreen || self.window_fullscreen,
            groups: self.tabs.groups(),
            split: self.split.as_ref().map(|s| SplitState {
                tabs: &s.tabs,
                layout: s.layout,
                shown: self.shown_split().is_some(),
                panes: self
                    .panes
                    .iter()
                    .map(|(id, rect)| Pane {
                        tab_id: *id,
                        rect: *rect,
                    })
                    .collect(),
            }),
        }
    }

    fn favicons(&self) -> HashMap<TabId, String> {
        self.tabs
            .tabs()
            .iter()
            .filter_map(|t| Some((t.id, t.favicon.clone()?)))
            .collect()
    }

    fn emit_shell<S: Serialize + Clone>(&self, event: &str, payload: S) {
        if let Err(e) = self
            .app
            .emit_to(EventTarget::webview(shell_label(self.id)), event, payload)
        {
            log::warn!("emit {event} failed: {e}");
        }
    }

    /// Publishes the snapshot if one is pending and allowed now (~30/s); otherwise returns
    /// how long to wait before trying again.
    fn flush(&mut self) -> Option<Duration> {
        if !self.state_dirty {
            return None;
        }
        let since = self.last_emit.elapsed();
        if since < Duration::from_millis(33) {
            return Some(Duration::from_millis(33) - since);
        }
        self.state_dirty = false;
        self.last_emit = Instant::now();
        self.emit_shell("browser:state", self.snapshot());
        None
    }

    // ---------- tab lifecycle ----------

    pub fn create_tab(
        &mut self,
        url: Option<String>,
        private: bool,
        activate: bool,
        opener: Option<TabId>,
    ) -> TabId {
        let id = self.tabs.allocate_id();
        let url = url.unwrap_or_else(|| NEW_TAB_URL.into());
        // Every tab of a private window is private, however it was opened.
        let private = private || self.private;
        let tab = Tab::new(id, &url, private);
        let is_web = tab.internal.is_none();
        self.tabs.insert(tab, opener);
        if is_web {
            self.load_url(id, &url);
        }
        if activate {
            self.activate(id);
        }
        self.mark_session_dirty();
        id
    }

    pub fn close_tab(&mut self, id: TabId) {
        let was_active = self.tabs.active_id() == Some(id);
        let Some(tab) = self.tabs.remove(id) else {
            return;
        };
        if tab.has_view {
            self.engine.destroy_view(id);
        }
        self.forget_tab(id, was_active);
    }

    /// True if `id` can move to a new window: it is here and is not the window's only tab.
    pub fn can_tear_off(&self, id: TabId) -> bool {
        self.tabs.len() > 1 && self.tabs.get(id).is_some()
    }

    /// Takes a tab out of this window, live view included, for a new window (tab tear-off).
    pub(super) fn detach_tab(&mut self, id: TabId) -> Option<DetachedTab> {
        let was_active = self.tabs.active_id() == Some(id);
        let mut tab = self.tabs.take(id)?;
        let view = match tab.has_view {
            true => {
                // The find bar stays here; its highlights must not travel with the page.
                self.engine.find_stop(id);
                self.engine.detach_view(id)
            }
            false => None,
        };
        // A view still being created cannot move; the new window creates its own.
        tab.has_view = view.is_some();
        tab.group_id = None;
        let download_navigation = self.download_navigations.contains(&id);
        self.forget_tab(id, was_active);
        Some(DetachedTab {
            tab,
            view,
            download_navigation,
        })
    }

    /// Takes in a tab detached from another window and shows it.
    pub(super) fn adopt_tab(&mut self, moved: DetachedTab) {
        let DetachedTab {
            mut tab,
            view,
            download_navigation,
        } = moved;
        let id = tab.id;
        if download_navigation {
            self.download_navigations.insert(id);
        }
        if let Some(view) = view {
            if let Err(e) = self.engine.attach_view(id, view) {
                log::warn!("moving the page to the new window failed: {e}");
                tab.has_view = false;
            }
        }
        if let Some(icon) = tab.favicon.clone() {
            self.emit_shell(
                "browser:favicon",
                serde_json::json!({ "tabId": id, "dataUrl": icon }),
            );
        }
        self.tabs.insert(tab, None);
        self.activate(id);
        self.mark_session_dirty();
    }

    /// Clean-up after a tab left this window (closed or moved).
    fn forget_tab(&mut self, id: TabId, was_active: bool) {
        if let Some(s) = self.split.as_mut() {
            s.tabs.retain(|t| *t != id);
            if s.tabs.len() < 2 {
                self.split = None;
            }
        }
        self.deny_prompts_for(id);
        self.pending_logins.remove(&id);
        self.download_navigations.remove(&id);
        if self.tabs.is_empty() {
            // Closing a window's last tab closes the window (and the app, if it was the last).
            self.close_requested = true;
            return;
        }
        // Only leaving the shown tab switches tabs; a background tab going away must not end
        // the active page's full screen or find session.
        match self.tabs.active_id() {
            Some(active) if was_active => self.activate(active),
            _ => self.relayout(),
        }
        self.mark_session_dirty();
    }

    pub fn close_others(&mut self, keep: TabId) {
        if self.tabs.get(keep).is_none() {
            return;
        }
        let others: Vec<TabId> = self
            .tabs
            .tabs()
            .iter()
            .filter(|t| t.id != keep && !t.pinned)
            .map(|t| t.id)
            .collect();
        // Activated first, so closing the others never activates (and loads) one of them.
        self.activate(keep);
        for id in others {
            self.close_tab(id);
        }
    }

    pub fn activate(&mut self, id: TabId) {
        let previous = self.tabs.active_id();
        if !self.tabs.activate(id) {
            return;
        }
        // Selecting a tab inside a collapsed group expands the group.
        if let Some(g) = self.tabs.get(id).and_then(|t| t.group_id) {
            if self.tabs.group(g).is_some_and(|g| g.collapsed) {
                self.tabs.update_group(g, None, None, Some(false), None);
            }
        }
        // Lazily create views for restored tabs the first time they are shown (all panes in split).
        let visible: Vec<TabId> = match &self.split {
            Some(s) if s.tabs.contains(&id) => s.tabs.clone(),
            _ => vec![id],
        };
        for t in visible {
            let needs_view = self
                .tabs
                .get(t)
                .is_some_and(|t| t.internal.is_none() && !t.has_view && t.error.is_none());
            if needs_view {
                let url = self.tabs.get(t).map(|t| t.url.clone()).unwrap_or_default();
                self.load_url(t, &url);
            }
        }
        if self.page_fullscreen {
            self.set_page_fullscreen(false);
        }
        // Find highlights belong to the page being left.
        if let Some(left) = previous.filter(|p| *p != id) {
            self.engine.find_stop(left);
        }
        self.relayout();
        self.mark_session_dirty();
    }

    pub fn activate_relative(&mut self, forward: bool) {
        if let Some(id) = self.tabs.cycle(forward) {
            self.activate(id);
        }
    }

    pub fn activate_index(&mut self, index: Option<usize>) {
        let id = match index {
            Some(i) => self.tabs.nth(i),
            None => self.tabs.last(),
        };
        if let Some(id) = id {
            self.activate(id);
        }
    }

    /// `group`: the tab was dropped onto that group and joins it.
    pub fn move_tab(&mut self, id: TabId, index: usize, group: Option<u64>) {
        match group {
            Some(g) => self.tabs.move_into_group(id, index, g),
            None => self.tabs.move_to(id, index),
        }
        self.mark_session_dirty();
    }

    pub fn duplicate(&mut self, id: TabId) {
        if let Some(t) = self.tabs.get(id) {
            let (url, private) = (t.url.clone(), t.private);
            self.create_tab(Some(url), private, true, Some(id));
        }
    }

    pub fn set_pinned(&mut self, id: TabId, pinned: bool) {
        self.tabs.set_pinned(id, pinned);
        self.mark_session_dirty();
    }

    pub fn reopen_closed(&mut self) {
        if let Some(c) = self.tabs.pop_closed() {
            let id = self.create_tab(Some(c.url), false, true, None);
            if let Some(t) = self.tabs.get_mut(id) {
                t.title = c.title;
            }
            if c.pinned {
                self.tabs.set_pinned(id, true);
            } else {
                self.tabs.move_to(id, c.index);
            }
        }
    }

    // ---------- navigation ----------

    /// Navigates a tab to address-bar input (URL, internal page or search terms).
    pub fn navigate(&mut self, id: TabId, input: &str) -> AppResult<()> {
        let engine = self.services.settings().search_engine;
        let url = resolve_input(input, engine)
            .ok_or_else(|| AppError::InvalidInput("nothing to open".into()))?;
        let tab = self
            .tabs
            .get_mut(id)
            .ok_or_else(|| AppError::NotFound("tab".into()))?;
        tab.error = None;
        if url::internal_page(&url).is_some() {
            tab.set_url(&url);
            tab.can_go_back = tab.has_view;
            self.relayout();
        } else {
            self.load_url(id, &url);
            self.engine.focus_view(id);
        }
        self.mark_session_dirty();
        Ok(())
    }

    /// Opens or reuses an internal page tab (History, Downloads, Settings...).
    pub fn open_internal(&mut self, url: &str) {
        let existing = self.tabs.tabs().iter().find(|t| t.url == url).map(|t| t.id);
        match existing {
            Some(id) => self.activate(id),
            None => {
                self.create_tab(Some(url.into()), false, true, None);
            }
        }
    }

    fn load_url(&mut self, id: TabId, url: &str) {
        let Some(tab) = self.tabs.get_mut(id) else {
            return;
        };
        let private = tab.private;
        tab.set_url(url);
        tab.loading = true;
        tab.error = None;
        let created = if tab.has_view {
            Ok(())
        } else {
            tab.has_view = true;
            self.engine.create_view(id, private, None)
        };
        let result = created.and_then(|_| self.engine.navigate(id, url));
        if let Err(e) = result {
            self.set_tab_error(
                id,
                TabError::Engine {
                    message: e.to_string(),
                },
            );
        }
        self.refresh_bookmark_flag(id);
        self.relayout();
        self.mark_dirty();
    }

    fn set_tab_error(&mut self, id: TabId, error: TabError) {
        if let Some(t) = self.tabs.get_mut(id) {
            t.error = Some(error);
            t.loading = false;
        }
        self.relayout();
        self.mark_dirty();
    }

    pub fn go_back(&mut self, id: TabId) -> AppResult<()> {
        let Some(tab) = self.tabs.get_mut(id) else {
            return Ok(());
        };
        // Leaving an internal page returns to the page still loaded in the view.
        if tab.internal.is_some() && tab.has_view {
            if let Some(web) = tab.web_url.clone() {
                tab.set_url(&web);
                self.refresh_bookmark_flag(id);
                self.relayout();
                self.mark_session_dirty();
                return Ok(());
            }
        }
        self.engine.go_back(id)
    }

    pub fn go_forward(&mut self, id: TabId) -> AppResult<()> {
        self.engine.go_forward(id)
    }

    pub fn reload(&mut self, id: TabId) -> AppResult<()> {
        let Some(tab) = self.tabs.get_mut(id) else {
            return Ok(());
        };
        if tab.internal.is_some() {
            return Ok(());
        }
        if let Some(TabError::LoadFailed { url, .. }) = tab.error.clone() {
            self.load_url(id, &url);
            return Ok(());
        }
        if matches!(tab.error, Some(TabError::Engine { .. })) || !tab.has_view {
            let url = tab.url.clone();
            self.load_url(id, &url);
            return Ok(());
        }
        tab.error = None;
        self.relayout();
        self.engine.reload(id)
    }

    pub fn stop(&mut self, id: TabId) -> AppResult<()> {
        self.engine.stop(id)
    }

    pub fn https_continue(&mut self, id: TabId) -> AppResult<()> {
        let Some(tab) = self.tabs.get_mut(id) else {
            return Ok(());
        };
        let Some(TabError::HttpsUnavailable { http_url }) = tab.error.take() else {
            return Ok(());
        };
        self.relayout();
        self.mark_dirty();
        self.engine.allow_http(id, &http_url)
    }

    pub fn zoom(&mut self, id: TabId, step: i32) -> AppResult<()> {
        const LEVELS: [f64; 13] = [
            0.25, 0.33, 0.5, 0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 2.0, 3.0,
        ];
        let Some(tab) = self.tabs.get_mut(id) else {
            return Ok(());
        };
        let current = tab.zoom;
        let next = match step {
            0 => 1.0,
            s if s > 0 => LEVELS
                .iter()
                .copied()
                .find(|l| *l > current + 0.001)
                .unwrap_or(3.0),
            _ => LEVELS
                .iter()
                .rev()
                .copied()
                .find(|l| *l < current - 0.001)
                .unwrap_or(0.25),
        };
        tab.zoom = next;
        self.mark_dirty();
        self.engine.set_zoom(id, next)
    }

    pub fn find(&mut self, text: &str, forward: bool) -> AppResult<()> {
        let Some(id) = self
            .tabs
            .active()
            .filter(|t| t.shows_web_content())
            .map(|t| t.id)
        else {
            return Err(AppError::InvalidInput(
                "find works on web pages only".into(),
            ));
        };
        if text.is_empty() {
            self.engine.find_stop(id);
            return Ok(());
        }
        self.engine.find(id, text, forward)
    }

    pub fn find_stop(&mut self) {
        if let Some(id) = self.tabs.active_id() {
            self.engine.find_stop(id);
        }
    }

    pub fn print(&mut self, id: TabId) -> AppResult<()> {
        self.engine.print(id)
    }

    pub fn focus_content(&mut self) {
        if let Some(id) = self
            .tabs
            .active()
            .filter(|t| t.shows_web_content())
            .map(|t| t.id)
        {
            self.engine.focus_view(id);
        }
    }

    pub fn active_tab_id(&self) -> Option<TabId> {
        self.tabs.active_id()
    }

    pub fn tab(&self, id: TabId) -> Option<&Tab> {
        self.tabs.get(id)
    }

    // ---------- layout ----------

    pub fn set_chrome_height(&mut self, css_px: f64) {
        self.chrome_height = css_px.clamp(0.0, 4000.0);
        self.relayout();
    }

    pub fn relayout(&mut self) {
        let Some(window) = self.app.get_window(&window_label(self.id)) else {
            return;
        };
        let (Ok(size), Ok(scale)) = (window.inner_size(), window.scale_factor()) else {
            return;
        };
        let full = self.page_fullscreen || self.window_fullscreen;
        let top = if full {
            0
        } else {
            (self.chrome_height * scale).round() as i32
        };
        let (w, h) = (size.width as i32, size.height as i32);
        let area = Rect {
            x: 0,
            y: top,
            width: w,
            height: (h - top).max(0),
        };
        // Content stays hidden until the shell has reported its chrome height.
        let ready = self.chrome_height > 0.0 || full;
        self.panes = match self.shown_split() {
            Some(s) => {
                let css = split::RectF {
                    x: 0.0,
                    y: 0.0,
                    width: w as f64 / scale,
                    height: area.height as f64 / scale,
                };
                s.tabs
                    .iter()
                    .copied()
                    .zip(split::pane_rects(s.layout, s.tabs.len(), css, split::GAP))
                    .collect()
            }
            None => Vec::new(),
        };
        let web = |t: &Tab| t.shows_web_content() && ready;
        let panes: Vec<(TabId, Rect)> = if self.panes.is_empty() {
            self.tabs
                .active()
                .filter(|t| web(t))
                .map(|t| {
                    (
                        t.id,
                        Rect {
                            x: 0,
                            y: 0,
                            width: w,
                            height: area.height,
                        },
                    )
                })
                .into_iter()
                .collect()
        } else {
            self.panes
                .iter()
                .filter(|(id, _)| self.tabs.get(*id).is_some_and(web))
                .map(|(id, r)| {
                    let px = |v: f64| (v * scale).round() as i32;
                    (
                        *id,
                        Rect {
                            x: px(r.x),
                            y: px(r.y),
                            width: px(r.x + r.width) - px(r.x),
                            height: px(r.y + r.height) - px(r.y),
                        },
                    )
                })
                .collect()
        };
        self.engine.layout(area, &panes);
        self.mark_dirty();
    }

    /// The split view, when the active tab is one of its panes.
    fn shown_split(&self) -> Option<&split::Split> {
        let active = self.tabs.active_id()?;
        self.split.as_ref().filter(|s| s.tabs.contains(&active))
    }

    // ---------- split view ----------

    /// Shows the active tab with `count - 1` other tabs side by side. Reuses the current split's
    /// tabs, then the tabs to the right of the active one, then new tabs.
    pub fn set_split(&mut self, count: usize, layout: split::SplitLayout) {
        let Some(active) = self.tabs.active_id() else {
            return;
        };
        let count = count.clamp(2, split::MAX_PANES);
        let private = self.tabs.get(active).is_some_and(|t| t.private);
        let mut panes = match &self.split {
            Some(s) if s.tabs.contains(&active) => s.tabs.clone(),
            _ => vec![active],
        };
        if panes.len() > count {
            // Keep the active pane plus the first others, in their current order.
            let mut others = count - 1;
            panes.retain(|t| {
                let keep = *t == active || others > 0;
                if *t != active && keep {
                    others -= 1;
                }
                keep
            });
        }
        let start = self.tabs.index_of(active).unwrap_or(0);
        let ordered: Vec<TabId> = {
            let all = self.tabs.tabs();
            all[start..]
                .iter()
                .chain(all[..start].iter())
                .map(|t| t.id)
                .collect()
        };
        for id in ordered {
            if panes.len() >= count {
                break;
            }
            if !panes.contains(&id)
                && self
                    .tabs
                    .get(id)
                    .is_some_and(|t| t.private == private && !t.pinned)
            {
                panes.push(id);
            }
        }
        while panes.len() < count {
            // Extra panes open as new tabs at the end (not inside a neighbouring tab group).
            let id = self.create_tab(None, private, false, None);
            panes.push(id);
        }
        self.split = Some(split::Split {
            tabs: panes,
            layout,
        });
        self.activate(active);
    }

    // ---------- tab groups ----------

    pub fn group_create(&mut self, tab: TabId) -> AppResult<u64> {
        let id = self
            .tabs
            .create_group(tab, "", None)
            .ok_or_else(|| AppError::InvalidInput("pinned tabs cannot be grouped".into()))?;
        self.mark_session_dirty();
        Ok(id)
    }

    pub fn group_assign(&mut self, tab: TabId, group: Option<u64>) {
        self.tabs.assign(tab, group);
        self.mark_session_dirty();
    }

    /// `icon`: a folder icon name, or `Some("")` to remove the icon.
    pub fn group_update(
        &mut self,
        id: u64,
        title: Option<&str>,
        color: Option<&str>,
        collapsed: Option<bool>,
        icon: Option<&str>,
    ) -> AppResult<()> {
        if let Some(c) = color.filter(|c| !tabs::valid_group_color(c)) {
            return Err(AppError::InvalidInput(format!("invalid colour '{c}'")));
        }
        if let Some(i) = icon.filter(|i| !i.is_empty() && !crate::storage::FOLDER_ICONS.contains(i))
        {
            return Err(AppError::InvalidInput(format!("unknown icon '{i}'")));
        }
        if collapsed == Some(true) {
            // The active tab cannot hide inside a collapsed group: move to the nearest outside tab.
            let active_inside = self.tabs.active().is_some_and(|t| t.group_id == Some(id));
            if active_inside {
                let members = self.tabs.group_tabs(id);
                let last = members
                    .last()
                    .and_then(|t| self.tabs.index_of(*t))
                    .unwrap_or(0);
                let first = members
                    .first()
                    .and_then(|t| self.tabs.index_of(*t))
                    .unwrap_or(0);
                let outside = self
                    .tabs
                    .nth(last + 1)
                    .or_else(|| first.checked_sub(1).and_then(|i| self.tabs.nth(i)));
                match outside {
                    Some(t) => self.activate(t),
                    None => {
                        return Err(AppError::InvalidInput(
                            "open another tab before collapsing this group".into(),
                        ))
                    }
                }
            }
        }
        if !self.tabs.update_group(id, title, color, collapsed, icon) {
            return Err(AppError::NotFound("tab group".into()));
        }
        self.mark_session_dirty();
        Ok(())
    }

    pub fn group_ungroup(&mut self, id: u64) {
        self.tabs.ungroup(id);
        self.mark_session_dirty();
    }

    pub fn group_close(&mut self, id: u64) {
        let mut tabs = self.tabs.group_tabs(id);
        // The active member goes last, so the closing members are never activated (and loaded).
        let active = self.tabs.active_id();
        tabs.sort_by_key(|t| Some(*t) == active);
        for tab in tabs {
            self.close_tab(tab);
        }
    }

    /// "Open in split view": adds a tab to the shown split (max four panes) or starts one.
    pub fn add_to_split(&mut self, tab: TabId) {
        let Some(active) = self.tabs.active_id() else {
            return;
        };
        // The same panes `set_split` would accept: an open, unpinned tab of the same privacy.
        let private = self.tabs.get(active).is_some_and(|t| t.private);
        if !self
            .tabs
            .get(tab)
            .is_some_and(|t| t.private == private && !t.pinned)
        {
            return;
        }
        // Already a pane: re-running `set_split` below would collapse a larger grid to two.
        if self.split.as_ref().is_some_and(|s| s.tabs.contains(&tab)) {
            return;
        }
        if tab == active {
            return self.set_split(2, split::SplitLayout::Columns);
        }
        match self.split.as_mut().filter(|s| s.tabs.contains(&active)) {
            Some(s) if s.tabs.len() < split::MAX_PANES => s.tabs.push(tab),
            Some(s) => {
                if let Some(i) = split::pane_to_replace(&s.tabs, active) {
                    s.tabs[i] = tab;
                }
            }
            None => {
                self.split = Some(split::Split {
                    tabs: vec![active, tab],
                    layout: split::SplitLayout::Columns,
                })
            }
        }
        self.activate(active);
    }

    pub fn close_split(&mut self) {
        self.split = None;
        self.relayout();
        self.mark_session_dirty();
    }

    pub fn notify_window_moved(&self) {
        self.engine.notify_window_moved();
    }

    /// The window moved or was resized: save its new place with the session.
    pub fn bounds_changed(&mut self) {
        self.session_dirty = true;
    }

    /// True once if the window was restored maximized (it is maximized when first shown).
    pub fn take_pending_maximize(&mut self) -> bool {
        std::mem::take(&mut self.pending_maximize)
    }

    /// Geometry and split view saved with this window's tabs.
    fn session_extras(&self) -> session::WindowExtras<'_> {
        let window = self.app.get_window(&window_label(self.id));
        let state = |f: fn(&tauri::Window) -> tauri::Result<bool>| {
            window.as_ref().and_then(|w| f(w).ok()).unwrap_or(false)
        };
        let maximized = state(tauri::Window::is_maximized);
        let normal = !maximized
            && !state(tauri::Window::is_minimized)
            && !self.window_fullscreen
            && !self.page_fullscreen;
        let bounds = window.filter(|_| normal).and_then(|w| {
            let (pos, size) = (w.outer_position().ok()?, w.inner_size().ok()?);
            Some(session::WindowBounds {
                x: pos.x,
                y: pos.y,
                width: size.width,
                height: size.height,
            })
        });
        session::WindowExtras {
            bounds,
            maximized,
            split: self.split.as_ref(),
        }
    }

    fn set_page_fullscreen(&mut self, on: bool) {
        self.page_fullscreen = on;
        if !self.window_fullscreen {
            if let Some(w) = self.app.get_window(&window_label(self.id)) {
                let _ = w.set_fullscreen(on);
            }
        }
        self.relayout();
        self.mark_dirty();
    }

    pub fn toggle_window_fullscreen(&mut self) {
        self.window_fullscreen = !self.window_fullscreen;
        if let Some(w) = self.app.get_window(&window_label(self.id)) {
            let _ = w.set_fullscreen(self.window_fullscreen || self.page_fullscreen);
        }
        self.relayout();
        self.mark_dirty();
    }

    // ---------- bookmarks ----------

    fn refresh_bookmark_flag(&mut self, id: TabId) {
        let Some(url) = self
            .tabs
            .get(id)
            .filter(|t| t.internal.is_none())
            .map(|t| t.url.clone())
        else {
            return;
        };
        let saved = self.services.db.bookmark_is_saved(&url).unwrap_or(false);
        if let Some(t) = self.tabs.get_mut(id) {
            t.bookmarked = saved;
        }
    }

    /// Bookmarks of the page's site take its icon (the caller checked `icon_is_current`). Private
    /// tabs teach nothing.
    fn learn_site_icon(&self, tab: TabId, page_url: &str, data_url: Option<&str>) {
        let (Some(t), Some(icon)) = (self.tabs.get(tab), data_url) else {
            return;
        };
        if t.private || url::origin_of(&t.url).is_none() {
            return;
        }
        match self.services.db.bookmark_set_site_favicon(page_url, icon) {
            Ok(0) => {}
            Ok(_) => {
                let _ = self.app.emit("bookmarks:changed", ());
            }
            Err(e) => log::warn!("saving a bookmark icon failed: {e}"),
        }
    }

    /// Re-reads bookmark flags for all tabs (after edits on the Bookmarks page).
    pub fn refresh_bookmarks(&mut self) {
        let ids: Vec<TabId> = self.tabs.tabs().iter().map(|t| t.id).collect();
        for id in ids {
            self.refresh_bookmark_flag(id);
        }
        self.mark_dirty();
    }

    /// "Bookmark this page" from the bar menu: adds the active page unless it is already saved.
    /// Returns whether a bookmark was added.
    pub fn bookmark_active_page(&mut self) -> AppResult<bool> {
        let Some(id) = self.tabs.active_id() else {
            return Ok(false);
        };
        let saved = self.tabs.get(id).is_some_and(|t| t.bookmarked);
        if saved {
            return Ok(false);
        }
        self.toggle_bookmark(id)
    }

    pub fn toggle_bookmark(&mut self, id: TabId) -> AppResult<bool> {
        let tab = self
            .tabs
            .get(id)
            .ok_or_else(|| AppError::NotFound("tab".into()))?;
        if tab.internal.is_some() || url::web_url(&tab.url).is_none() {
            return Err(AppError::InvalidInput(
                "only web pages can be bookmarked".into(),
            ));
        }
        let title = if tab.title.is_empty() {
            tab.url.clone()
        } else {
            tab.title.clone()
        };
        let saved =
            self.services
                .db
                .bookmark_toggle(&tab.url.clone(), &title, tab.favicon.as_deref())?;
        self.refresh_bookmarks();
        Ok(saved)
    }

    // ---------- prompts ----------

    fn push_prompt(&mut self, make: impl FnOnce(u64) -> Prompt) {
        self.next_prompt += 1;
        let prompt = make(self.next_prompt);
        // Bars without a pending engine request: a page repeating one replaces its bar instead
        // of stacking bars without end.
        if matches!(
            prompt,
            Prompt::PopupBlocked { .. }
                | Prompt::ExternalProtocol { .. }
                | Prompt::SavePassword { .. }
        ) {
            let (tab, kind) = (prompt.tab_id(), std::mem::discriminant(&prompt));
            self.prompts
                .retain(|p| p.tab_id() != tab || std::mem::discriminant(p) != kind);
        }
        self.prompts.push(prompt);
        self.mark_dirty();
    }

    fn take_prompt(&mut self, id: u64) -> Option<Prompt> {
        let i = self.prompts.iter().position(|p| p.id() == id)?;
        self.mark_dirty();
        Some(self.prompts.remove(i))
    }

    fn deny_prompts_for(&mut self, tab: TabId) {
        let (theirs, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut self.prompts)
            .into_iter()
            .partition(|p| p.tab_id() == tab);
        self.prompts = keep;
        for p in theirs {
            if let Prompt::Permission { request, .. } = p {
                self.engine.resolve_permission(request, false);
            }
        }
        self.mark_dirty();
    }

    pub fn respond_prompt(&mut self, id: u64, allow: bool, remember: bool) -> AppResult<()> {
        let prompt = self
            .take_prompt(id)
            .ok_or_else(|| AppError::NotFound("prompt".into()))?;
        match prompt {
            Prompt::Permission {
                tab_id,
                origin,
                kind,
                request,
                ..
            } => {
                // Answered first: the page must get its answer even if saving the choice fails.
                self.engine.resolve_permission(request, allow);
                let private = self.tabs.get(tab_id).is_some_and(|t| t.private);
                if remember && !private {
                    let d = if allow {
                        Decision::Allow
                    } else {
                        Decision::Deny
                    };
                    if let Err(e) = permissions::remember(&self.services.db, &origin, kind, Some(d))
                    {
                        log::error!("saving a site permission failed: {e}");
                        return Err(e);
                    }
                }
            }
            Prompt::SavePassword {
                origin,
                username,
                password,
                ..
            } => self.answer_save_login(&origin, &username, &password, allow, remember)?,
            Prompt::ExternalProtocol { uri, .. } => {
                if allow {
                    use tauri_plugin_opener::OpenerExt;
                    self.app
                        .opener()
                        .open_url(&uri, None::<&str>)
                        .map_err(|e| {
                            AppError::Engine(format!("could not open external app: {e}"))
                        })?;
                }
            }
            Prompt::PopupBlocked {
                tab_id,
                origin,
                url,
                ..
            } => {
                if allow {
                    let private = self.tabs.get(tab_id).is_some_and(|t| t.private);
                    if remember && !private {
                        permissions::remember(
                            &self.services.db,
                            &origin,
                            PermissionKind::Popups,
                            Some(Decision::Allow),
                        )?;
                    }
                    self.create_tab(Some(url), private, true, Some(tab_id));
                }
            }
        }
        Ok(())
    }

    // ---------- engine events ----------

    fn handle_event(&mut self, event: EngineEvent) {
        use EngineEvent as E;
        match event {
            E::ViewReady { tab } => {
                let zoom = self.tabs.get(tab).map(|t| t.zoom).unwrap_or(1.0);
                if (zoom - 1.0).abs() > f64::EPSILON {
                    let _ = self.engine.set_zoom(tab, zoom);
                }
                self.relayout();
            }
            E::ViewFailed { tab, error } => {
                if let Some(t) = self.tabs.get_mut(tab) {
                    t.has_view = false;
                }
                self.set_tab_error(tab, TabError::Engine { message: error });
            }
            E::NavigationStarting { tab, url } => {
                self.offer_pending_login(tab);
                let old_origin = self.tabs.get(tab).and_then(|t| url::origin_of(&t.url));
                if let Some(t) = self.tabs.get_mut(tab) {
                    t.loading = true;
                    t.blocked = 0;
                    t.saved_logins = 0;
                    t.error = None;
                    if t.internal.is_none() {
                        t.set_url(&url);
                    } else {
                        t.web_url = Some(url.clone());
                    }
                }
                if old_origin != url::origin_of(&url) {
                    let stale: Vec<u64> = self
                        .prompts
                        .iter()
                        .filter(|p| p.tab_id() == tab && !matches!(p, Prompt::SavePassword { .. }))
                        .map(|p| p.id())
                        .collect();
                    for id in stale {
                        let _ = self.respond_prompt(id, false, false);
                    }
                }
                self.refresh_bookmark_flag(tab);
                self.mark_dirty();
            }
            E::SourceChanged { tab, url } => {
                if let Some(t) = self.tabs.get_mut(tab) {
                    if t.internal.is_none() {
                        t.set_url(&url);
                    } else {
                        t.web_url = Some(url);
                    }
                }
                self.refresh_bookmark_flag(tab);
                self.mark_session_dirty();
            }
            E::TitleChanged { tab, title } => {
                if let Some(t) = self.tabs.get_mut(tab) {
                    t.web_title = title.clone();
                    if t.internal.is_none() {
                        t.title = title.clone();
                        if !t.private && url::web_url(&t.url).is_some() {
                            let _ = self.services.db.history_update_title(&t.url, &title);
                            self.history_dirty = true;
                        }
                    }
                }
                self.mark_session_dirty();
            }
            E::HistoryChanged {
                tab,
                can_go_back,
                can_go_forward,
            } => {
                if let Some(t) = self.tabs.get_mut(tab) {
                    t.can_go_back = can_go_back;
                    t.can_go_forward = can_go_forward;
                }
                self.mark_dirty();
            }
            E::NavigationCompleted {
                tab,
                success,
                committed_url,
            } => {
                // The page's submit message and the navigation it causes travel on separate
                // channels; if the message lost the race with NavigationStarting, offer it now.
                self.offer_pending_login(tab);
                if let Some(t) = self.tabs.get_mut(tab) {
                    t.loading = false;
                    // A navigation that did not commit (download, cancel, stop) must not leave
                    // its URL in the address bar or the saved session.
                    let reverted = !success
                        && t.error.is_none()
                        && t.internal.is_none()
                        && url::web_url(&committed_url).is_some();
                    if reverted && committed_url != t.url {
                        t.set_url(&committed_url);
                        self.session_dirty = true;
                    } else if !success
                        && t.error.is_none()
                        && self.download_navigations.contains(&tab)
                        && url::web_url(&committed_url).is_none()
                    {
                        // A fresh tab that only produced a download shows the new-tab page.
                        t.set_url(NEW_TAB_URL);
                        self.session_dirty = true;
                    }
                    self.download_navigations.remove(&tab);
                    // History is local-only and never recorded for private tabs.
                    if success
                        && !t.private
                        && t.internal.is_none()
                        && url::web_url(&t.url).is_some()
                    {
                        match self.services.db.history_record_visit(&t.url, &t.title) {
                            Ok(_) => self.history_dirty = true,
                            Err(e) => log::error!("recording history failed: {e}"),
                        }
                    }
                }
                self.mark_dirty();
            }
            E::LoadFailed { tab, url, reason } => self.set_tab_error(
                tab,
                TabError::LoadFailed {
                    url,
                    reason: reason.into(),
                },
            ),
            E::HttpsUpgradeFailed { tab, http_url } => {
                self.set_tab_error(tab, TabError::HttpsUnavailable { http_url })
            }
            E::FaviconChanged {
                tab,
                page_url,
                data_url,
            } => {
                // Late icons of the page the tab has since left are dropped.
                let changed = match self.tabs.get_mut(tab) {
                    Some(t) if icon_is_current(t, &page_url) => {
                        let changed = t.favicon != data_url;
                        t.favicon = data_url.clone();
                        changed
                    }
                    _ => return,
                };
                // The same icon again would only re-decode the PNG for bookmarks on the UI thread.
                if changed {
                    self.learn_site_icon(tab, &page_url, data_url.as_deref());
                }
                #[derive(Serialize, Clone)]
                #[serde(rename_all = "camelCase")]
                struct Favicon {
                    tab_id: TabId,
                    data_url: Option<String>,
                }
                self.emit_shell(
                    "browser:favicon",
                    Favicon {
                        tab_id: tab,
                        data_url,
                    },
                );
            }
            E::ZoomChanged { tab, zoom } => {
                if let Some(t) = self.tabs.get_mut(tab) {
                    t.zoom = zoom;
                }
                self.mark_dirty();
            }
            E::RequestBlocked { tab } => {
                if let Some(t) = self.tabs.get_mut(tab) {
                    t.blocked += 1;
                }
                // Only the active tab's count is shown (switching tabs re-sends the state).
                if self.tabs.active_id() == Some(tab) {
                    self.mark_dirty();
                }
            }
            E::LoginFormFound { tab, origin } => self.login_form_found(tab, &origin),
            E::LoginSubmitted {
                tab,
                origin,
                username,
                password,
            } => self.login_submitted(tab, origin, username, password),
            E::LoginSucceeded { tab } => self.offer_pending_login(tab),
            E::NewWindowRequested {
                opener,
                request,
                url,
                user_initiated,
                background,
            } => self.on_new_window(opener, request, url, user_initiated, background),
            E::OpenLink {
                opener,
                url,
                private,
            } => {
                if url::web_url(&url).is_some() {
                    let private = private || self.tabs.get(opener).is_some_and(|t| t.private);
                    self.create_tab(Some(url), private, false, Some(opener));
                }
            }
            E::CloseRequested { tab } => self.close_tab(tab),
            E::FullscreenChanged { tab, fullscreen } => {
                if self.tabs.active_id() == Some(tab) {
                    self.set_page_fullscreen(fullscreen);
                }
            }
            E::PermissionRequested {
                tab,
                request,
                origin,
                kind,
            } => {
                let Some(t) = self.tabs.get(tab) else {
                    return self.engine.resolve_permission(request, false);
                };
                let stored = if t.private {
                    None
                } else {
                    permissions::stored(&self.services.db, &origin, kind)
                        .ok()
                        .flatten()
                };
                match stored {
                    Some(d) => self
                        .engine
                        .resolve_permission(request, d == Decision::Allow),
                    None => self.push_prompt(|id| Prompt::Permission {
                        id,
                        tab_id: tab,
                        origin,
                        kind,
                        request,
                    }),
                }
            }
            E::ExternalProtocolRequested { tab, uri } => {
                self.push_prompt(|id| Prompt::ExternalProtocol {
                    id,
                    tab_id: tab,
                    uri,
                });
            }
            // Downloads are shared by all windows; `Browser::handle_event` takes them.
            E::DownloadStarted { .. } | E::DownloadProgress { .. } | E::DownloadFinished { .. } => {
            }
            E::FindResult {
                tab,
                matches,
                active,
            } => {
                #[derive(Serialize, Clone)]
                #[serde(rename_all = "camelCase")]
                struct FindResult {
                    tab_id: TabId,
                    matches: i32,
                    active: i32,
                }
                self.emit_shell(
                    "find:result",
                    FindResult {
                        tab_id: tab,
                        matches,
                        active,
                    },
                );
            }
            E::Shortcut { action, .. } => {
                // Actions that type into the UI need keyboard focus moved out of the page first.
                if matches!(
                    action,
                    "focusAddressBar" | "findInPage" | "newTab" | "newPrivateTab"
                ) {
                    if let Some(shell) = self.app.get_webview(&shell_label(self.id)) {
                        let _ = shell.set_focus();
                    }
                }
                self.emit_shell("shell:action", action)
            }
            E::FocusLeft { .. } => self.focus_shell(),
            E::ViewFocused { tab } => {
                // Clicking into a split pane makes it the active tab (address bar follows it).
                let in_split = self.shown_split().is_some_and(|s| s.tabs.contains(&tab));
                let left = self.tabs.active_id();
                if in_split && left != Some(tab) {
                    self.tabs.activate(tab);
                    // Find highlights belong to the pane being left.
                    if let Some(left) = left {
                        self.engine.find_stop(left);
                    }
                    self.mark_session_dirty();
                }
            }
            E::Crashed { tab } => self.set_tab_error(tab, TabError::Crashed),
        }
    }

    fn on_new_window(
        &mut self,
        opener: TabId,
        request: RequestId,
        url: String,
        user_initiated: bool,
        background: bool,
    ) {
        let Some(private) = self.tabs.get(opener).map(|t| t.private) else {
            self.engine.deny_popup(request);
            return;
        };
        let origin = self.tabs.get(opener).and_then(|t| url::origin_of(&t.url));
        let allowed = user_initiated
            || (!private
                && origin.as_deref().is_some_and(|o| {
                    permissions::stored(&self.services.db, o, PermissionKind::Popups)
                        .ok()
                        .flatten()
                        == Some(Decision::Allow)
                }));
        if !allowed {
            self.engine.deny_popup(request);
            if let (Some(origin), true) = (origin, url::web_url(&url).is_some()) {
                self.push_prompt(|id| Prompt::PopupBlocked {
                    id,
                    tab_id: opener,
                    origin,
                    url,
                });
            }
            return;
        }
        let id = self.tabs.allocate_id();
        // Only web addresses: `veyscape://settings` here would show privileged UI in the new tab.
        // The view reports where it really goes once it navigates.
        let start = match url::web_url(&url) {
            Some(_) => url.as_str(),
            None => "about:blank",
        };
        let mut tab = Tab::new(id, start, private);
        tab.has_view = true;
        tab.loading = true;
        self.tabs.insert(tab, Some(opener));
        if let Err(e) = self.engine.create_view(id, private, Some(request)) {
            self.engine.deny_popup(request);
            self.set_tab_error(
                id,
                TabError::Engine {
                    message: e.to_string(),
                },
            );
        }
        if !background {
            self.activate(id);
        }
        self.mark_session_dirty();
    }

    /// Puts keyboard focus where the user expects it after the window is activated:
    /// the page for web content, otherwise the browser UI.
    pub fn restore_focus(&self) {
        // Only redirect focus that is not in this window's webviews; a click may already have placed it.
        if !self.engine.focus_needs_restoring() {
            return;
        }
        match self.tabs.active().filter(|t| t.shows_web_content()) {
            Some(t) => self.engine.focus_view(t.id),
            None => {
                if let Some(shell) = self.app.get_webview(&shell_label(self.id)) {
                    let _ = shell.set_focus();
                }
            }
        }
    }

    /// Moves keyboard focus from web content back to the browser UI's address bar.
    pub fn focus_shell(&self) {
        if let Some(shell) = self.app.get_webview(&shell_label(self.id)) {
            let _ = shell.set_focus();
        }
        self.emit_shell("shell:action", "focusAddressBar");
    }
}
