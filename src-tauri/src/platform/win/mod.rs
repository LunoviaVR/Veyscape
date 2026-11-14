//! Windows adapter: raw WebView2 controllers hosted in a child HWND ("content host").
//!
//! Why not Tauri child webviews for tabs? Tauri injects `window.isTauri`, `__TAURI_INTERNALS__`
//! and the invoke key into every webview it creates. That would expose an IPC entry point and a
//! fingerprinting signal to every website. Raw controllers have neither.
//! See docs/decisions/0001-raw-webview2-tabs.md.
//!
//! Threading: everything here runs on the UI thread. COM pointers are cloned out of the
//! `RefCell`s before calling into WebView2, because WebView2 may raise events synchronously.

mod content_host;
pub use content_host::set_overlay_window;
pub use handlers::restrict_ui_context_menu;
mod handlers;
mod passwords;
mod util;

use super::*;
use crate::errors::AppError;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::*;
use windows::core::{Interface, HSTRING};
use windows::Win32::Foundation::{HWND, RECT};

/// Per-view state read by the event handlers (separate from the view map to avoid re-borrowing).
#[derive(Default)]
pub(super) struct ViewState {
    pub tab: TabId,
    pub private: bool,
    /// Top-level URL of the pending (or current) navigation: its document request is not filtered.
    pub top_url: RefCell<String>,
    /// URL of the document the page shows (set when it commits); the "source" for filtering its
    /// requests, which go on while a new navigation is pending.
    pub page_url: RefCell<String>,
    /// Navigation id `top_url` belongs to.
    pub top_nav: Cell<u64>,
    /// (navigation id, original http URL) for an in-flight HTTPS-only upgrade.
    pub upgrade: RefCell<Option<(u64, String)>>,
    pub http_allowed: RefCell<HashSet<String>>,
    pub cosmetic_exceptions: RefCell<HashSet<String>>,
    pub generic_cosmetic: Cell<bool>,
    /// Set when a navigation turned into a download, so its "failure" is not shown as an error.
    pub download_started: Cell<bool>,
    /// Execution context of the password script in the current document (0 = none).
    pub login_context: Cell<i64>,
}

pub(super) struct View {
    controller: ICoreWebView2Controller,
    core: ICoreWebView2,
    state: std::rc::Rc<ViewState>,
    find: Option<ICoreWebView2Find>,
    find_term: String,
}

enum Slot {
    Creating { pending_url: Option<String> },
    Ready(View),
}

type PopupMap = HashMap<
    RequestId,
    (
        ICoreWebView2NewWindowRequestedEventArgs,
        ICoreWebView2Deferral,
    ),
>;
type PermissionMap = HashMap<
    RequestId,
    (
        ICoreWebView2PermissionRequestedEventArgs,
        ICoreWebView2Deferral,
    ),
>;
type DownloadMap = HashMap<i64, ICoreWebView2DownloadOperation>;

/// Requests waiting for the browser core, one set for every window's engine (UI thread only): a
/// tab moved to another window keeps reporting through the engine it was created in, and its
/// popups, permission prompts and downloads must still resolve in the new one.
#[derive(Default, Clone)]
struct Requests {
    popups: std::rc::Rc<RefCell<PopupMap>>,
    permissions: std::rc::Rc<RefCell<PermissionMap>>,
    downloads: std::rc::Rc<RefCell<DownloadMap>>,
    next: std::rc::Rc<Cell<RequestId>>,
}

thread_local! {
    static REQUESTS: Requests = Requests::default();
}

/// State shared with COM callbacks.
pub(super) struct Shared {
    pub hooks: EngineHooks,
    pub env: ICoreWebView2Environment,
    pub host: HWND,
    views: RefCell<HashMap<TabId, Slot>>,
    pub popups: std::rc::Rc<RefCell<PopupMap>>,
    pub permissions: std::rc::Rc<RefCell<PermissionMap>>,
    pub downloads: std::rc::Rc<RefCell<DownloadMap>>,
    next_request: std::rc::Rc<Cell<RequestId>>,
    /// Where each visible view goes, relative to the content host. Absent = hidden.
    placements: RefCell<HashMap<TabId, Rect>>,
    area: Cell<Rect>,
    prefs: RefCell<Option<ProfilePrefs>>,
}

impl Shared {
    pub fn emit(&self, event: EngineEvent) {
        (self.hooks.sink)(event)
    }

    pub fn next_request_id(&self) -> RequestId {
        let id = self.next_request.get() + 1;
        self.next_request.set(id);
        id
    }
}

pub struct WinEngine {
    shared: std::rc::Rc<Shared>,
}

impl WinEngine {
    /// `env` is the shell webview's environment: tabs share its browser process but use a
    /// separate profile ("Browsing"), so cookies and storage never mix with the UI's.
    pub fn new(
        window: &tauri::Window,
        env: ICoreWebView2Environment,
        hooks: EngineHooks,
    ) -> AppResult<Self> {
        let parent = window.hwnd()?;
        let host = content_host::create(parent)?;
        let requests = REQUESTS.with(Requests::clone);
        Ok(WinEngine {
            shared: std::rc::Rc::new(Shared {
                hooks,
                env,
                host,
                views: RefCell::default(),
                popups: requests.popups,
                permissions: requests.permissions,
                downloads: requests.downloads,
                next_request: requests.next,
                placements: RefCell::default(),
                area: Cell::new(Rect::default()),
                prefs: RefCell::new(None),
            }),
        })
    }

    fn core(&self, tab: TabId) -> AppResult<ICoreWebView2> {
        match self.shared.views.borrow().get(&tab) {
            Some(Slot::Ready(v)) => Ok(v.core.clone()),
            _ => Err(AppError::Engine("view not ready".into())),
        }
    }

    fn controller(&self, tab: TabId) -> Option<ICoreWebView2Controller> {
        match self.shared.views.borrow().get(&tab) {
            Some(Slot::Ready(v)) => Some(v.controller.clone()),
            _ => None,
        }
    }

    fn view_state(&self, tab: TabId) -> Option<std::rc::Rc<ViewState>> {
        match self.shared.views.borrow().get(&tab) {
            Some(Slot::Ready(v)) => Some(v.state.clone()),
            _ => None,
        }
    }

    fn any_normal_core(&self) -> Option<ICoreWebView2> {
        self.shared.views.borrow().values().find_map(|s| match s {
            Slot::Ready(v) if !v.state.private => Some(v.core.clone()),
            _ => None,
        })
    }
}

/// Called when a controller finishes creating: configure it, then either adopt a popup or
/// perform the navigation queued while it was being created.
fn on_controller_created(
    shared: &std::rc::Rc<Shared>,
    tab: TabId,
    private: bool,
    popup: Option<RequestId>,
    controller: ICoreWebView2Controller,
) -> AppResult<()> {
    // The tab may have been closed, detached or its window closed while its view was being created.
    if !matches!(shared.views.borrow().get(&tab), Some(Slot::Creating { .. })) {
        if let Some(request) = popup {
            deny_popup(shared, request);
        }
        unsafe { controller.Close()? };
        return Ok(());
    }
    let core = unsafe { controller.CoreWebView2()? };
    let state = std::rc::Rc::new(ViewState {
        tab,
        private,
        generic_cosmetic: Cell::new(false),
        ..Default::default()
    });
    util::configure_settings(&core)?;
    if let Some(prefs) = shared.prefs.borrow().clone() {
        util::apply_profile_prefs(&core, &prefs.for_view(private));
    }
    handlers::register(shared, &controller, &core, &state)?;
    passwords::register(shared, &core, &state)?;

    let area = shared.area.get();
    let placement = shared.placements.borrow().get(&tab).copied();
    let b = placement.unwrap_or(Rect {
        x: 0,
        y: 0,
        width: area.width,
        height: area.height,
    });
    unsafe {
        if let Ok(c2) = controller.cast::<ICoreWebView2Controller2>() {
            // Transparent until first paint would flash; use the theme-neutral page default.
            let _ = c2.SetDefaultBackgroundColor(COREWEBVIEW2_COLOR {
                A: 255,
                R: 255,
                G: 255,
                B: 255,
            });
        }
        controller.SetBounds(to_rect(b))?;
        controller.SetIsVisible(placement.is_some())?;
    }

    let pending_url = match shared.views.borrow_mut().insert(
        tab,
        Slot::Ready(View {
            controller,
            core: core.clone(),
            state,
            find: None,
            find_term: String::new(),
        }),
    ) {
        Some(Slot::Creating { pending_url }) => pending_url,
        _ => None,
    };

    if let Some(request) = popup {
        // Taken out first: `if let` would keep the borrow alive across the WebView2 calls.
        let entry = shared.popups.borrow_mut().remove(&request);
        if let Some((args, deferral)) = entry {
            unsafe {
                args.SetNewWindow(&core)?;
                deferral.Complete()?;
            }
        }
    } else if let Some(url) = pending_url {
        unsafe { core.Navigate(&HSTRING::from(url))? };
    }
    shared.emit(EngineEvent::ViewReady { tab });
    Ok(())
}

impl BrowserEngine for WinEngine {
    fn create_view(
        &mut self,
        tab: TabId,
        private: bool,
        popup: Option<RequestId>,
    ) -> AppResult<()> {
        if self.shared.views.borrow().contains_key(&tab) {
            return Ok(());
        }
        self.shared
            .views
            .borrow_mut()
            .insert(tab, Slot::Creating { pending_url: None });
        let env10: ICoreWebView2Environment10 = self.shared.env.cast()?;
        let shared = self.shared.clone();
        let handler = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(
            move |result, controller| {
                let outcome = result
                    .map_err(AppError::from)
                    .and_then(|_| {
                        controller.ok_or_else(|| AppError::Engine("no controller".into()))
                    })
                    .and_then(|c| on_controller_created(&shared, tab, private, popup, c));
                if let Err(e) = outcome {
                    log::error!("creating view for tab {tab} failed: {e}");
                    shared.views.borrow_mut().remove(&tab);
                    if let Some(request) = popup {
                        deny_popup(&shared, request);
                    }
                    shared.emit(EngineEvent::ViewFailed {
                        tab,
                        error: e.to_string(),
                    });
                }
                Ok(())
            },
        ));
        unsafe {
            let options = env10.CreateCoreWebView2ControllerOptions()?;
            options.SetProfileName(&HSTRING::from("Browsing"))?;
            options.SetIsInPrivateModeEnabled(private)?;
            env10.CreateCoreWebView2ControllerWithOptions(self.shared.host, &options, &handler)?;
        }
        Ok(())
    }

    fn destroy_view(&mut self, tab: TabId) {
        let slot = self.shared.views.borrow_mut().remove(&tab);
        self.shared.placements.borrow_mut().remove(&tab);
        if let Some(Slot::Ready(view)) = slot {
            // Close() releases the renderer; InPrivate data is wiped when the last private view closes.
            unsafe {
                let _ = view.controller.Close();
            }
        }
    }

    fn navigate(&mut self, tab: TabId, url: &str) -> AppResult<()> {
        if let Some(Slot::Creating { pending_url }) = self.shared.views.borrow_mut().get_mut(&tab) {
            *pending_url = Some(url.to_string());
            return Ok(());
        }
        let core = self.core(tab)?;
        unsafe { core.Navigate(&HSTRING::from(url))? };
        Ok(())
    }

    fn reload(&self, tab: TabId) -> AppResult<()> {
        unsafe { Ok(self.core(tab)?.Reload()?) }
    }

    fn stop(&self, tab: TabId) -> AppResult<()> {
        unsafe { Ok(self.core(tab)?.Stop()?) }
    }

    fn go_back(&self, tab: TabId) -> AppResult<()> {
        unsafe { Ok(self.core(tab)?.GoBack()?) }
    }

    fn go_forward(&self, tab: TabId) -> AppResult<()> {
        unsafe { Ok(self.core(tab)?.GoForward()?) }
    }

    fn set_zoom(&self, tab: TabId, factor: f64) -> AppResult<()> {
        if let Some(c) = self.controller(tab) {
            unsafe { c.SetZoomFactor(factor)? };
        }
        Ok(())
    }

    fn layout(&mut self, area: Rect, panes: &[(TabId, Rect)]) {
        self.shared.area.set(area);
        content_host::set_bounds(self.shared.host, area);
        *self.shared.placements.borrow_mut() = panes.iter().copied().collect();
        let views: Vec<(TabId, ICoreWebView2Controller)> = self
            .shared
            .views
            .borrow()
            .iter()
            .filter_map(|(t, s)| match s {
                Slot::Ready(v) => Some((*t, v.controller.clone())),
                _ => None,
            })
            .collect();
        for (tab, c) in views {
            let placement = panes.iter().find(|(t, _)| *t == tab).map(|(_, r)| *r);
            unsafe {
                if let Some(r) = placement {
                    let _ = c.SetBounds(to_rect(r));
                }
                let _ = c.SetIsVisible(placement.is_some());
            }
        }
        // A single full-size pane needs no region; split view clips the host to its web panes so
        // the shell shows through gaps and panes that display internal pages.
        let full = panes.len() == 1
            && panes[0].1
                == Rect {
                    x: 0,
                    y: 0,
                    width: area.width,
                    height: area.height,
                };
        let rects: Vec<Rect> = panes.iter().map(|(_, r)| *r).collect();
        content_host::set_region(self.shared.host, if full { None } else { Some(&rects) });
        content_host::set_visible(self.shared.host, !panes.is_empty());
    }

    fn focus_view(&self, tab: TabId) {
        if let Some(c) = self.controller(tab) {
            unsafe {
                let _ = c.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC);
            }
        }
    }

    fn find(&mut self, tab: TabId, text: &str, forward: bool) -> AppResult<()> {
        let (core, existing, same_term) = match self.shared.views.borrow().get(&tab) {
            Some(Slot::Ready(v)) => (v.core.clone(), v.find.clone(), v.find_term == text),
            _ => return Err(AppError::Engine("view not ready".into())),
        };
        let find = match existing {
            Some(f) => f,
            None => {
                let f = handlers::create_find(&self.shared, &core, tab)?;
                if let Some(Slot::Ready(v)) = self.shared.views.borrow_mut().get_mut(&tab) {
                    v.find = Some(f.clone());
                }
                f
            }
        };
        unsafe {
            if same_term {
                if forward {
                    find.FindNext()?
                } else {
                    find.FindPrevious()?
                }
            } else {
                let env15: ICoreWebView2Environment15 = self.shared.env.cast()?;
                let options = env15.CreateFindOptions()?;
                options.SetFindTerm(&HSTRING::from(text))?;
                options.SetSuppressDefaultFindDialog(true)?;
                options.SetShouldHighlightAllMatches(true)?;
                find.Start(
                    &options,
                    &FindStartCompletedHandler::create(Box::new(|_| Ok(()))),
                )?;
                if let Some(Slot::Ready(v)) = self.shared.views.borrow_mut().get_mut(&tab) {
                    v.find_term = text.to_string();
                }
            }
        }
        Ok(())
    }

    fn find_stop(&mut self, tab: TabId) {
        let find = match self.shared.views.borrow_mut().get_mut(&tab) {
            Some(Slot::Ready(v)) => {
                v.find_term.clear();
                v.find.clone()
            }
            _ => None,
        };
        if let Some(f) = find {
            unsafe {
                let _ = f.Stop();
            }
        }
    }

    fn print(&self, tab: TabId) -> AppResult<()> {
        let core16: ICoreWebView2_16 = self.core(tab)?.cast()?;
        unsafe { core16.ShowPrintUI(COREWEBVIEW2_PRINT_DIALOG_KIND_SYSTEM)? };
        Ok(())
    }

    fn resolve_permission(&mut self, request: RequestId, allow: bool) {
        // Taken out first: `if let` would keep the borrow alive across the WebView2 calls.
        let entry = self.shared.permissions.borrow_mut().remove(&request);
        if let Some((args, deferral)) = entry {
            let state = if allow {
                COREWEBVIEW2_PERMISSION_STATE_ALLOW
            } else {
                COREWEBVIEW2_PERMISSION_STATE_DENY
            };
            unsafe {
                let _ = args.SetState(state);
                let _ = deferral.Complete();
            }
        }
    }

    fn deny_popup(&mut self, request: RequestId) {
        deny_popup(&self.shared, request);
    }

    fn allow_http(&mut self, tab: TabId, http_url: &str) -> AppResult<()> {
        let state = self
            .view_state(tab)
            .ok_or_else(|| AppError::Engine("view not ready".into()))?;
        if let Some(host) = crate::browser::url::host_of(http_url) {
            state.http_allowed.borrow_mut().insert(host);
        }
        self.navigate(tab, http_url)
    }

    fn cancel_download(&mut self, id: i64) -> AppResult<()> {
        let op = self.shared.downloads.borrow().get(&id).cloned();
        let op = op.ok_or_else(|| AppError::NotFound("active download".into()))?;
        unsafe { op.Cancel()? };
        Ok(())
    }

    fn resume_download(&mut self, id: i64) -> AppResult<()> {
        let op = self.shared.downloads.borrow().get(&id).cloned();
        let op = op.ok_or_else(|| {
            AppError::Unsupported("downloads from previous sessions cannot be resumed")
        })?;
        unsafe {
            let mut can = windows::core::BOOL::default();
            op.CanResume(&mut can)?;
            if !can.as_bool() {
                return Err(AppError::Unsupported(
                    "the server does not allow resuming this download",
                ));
            }
            op.Resume()?;
        }
        Ok(())
    }

    fn forget_download(&mut self, id: i64) {
        // Bound, so the operation is released after the borrow ends.
        let _op = self.shared.downloads.borrow_mut().remove(&id);
    }

    fn clear_data(&mut self, kinds: ClearDataKinds, done: Box<dyn FnOnce(AppResult<()>)>) {
        let mut mask = 0;
        if kinds.cookies {
            mask |= COREWEBVIEW2_BROWSING_DATA_KINDS_COOKIES.0
                | COREWEBVIEW2_BROWSING_DATA_KINDS_ALL_DOM_STORAGE.0
                | COREWEBVIEW2_BROWSING_DATA_KINDS_SERVICE_WORKERS.0;
        }
        if kinds.cache {
            mask |= COREWEBVIEW2_BROWSING_DATA_KINDS_DISK_CACHE.0
                | COREWEBVIEW2_BROWSING_DATA_KINDS_CACHE_STORAGE.0;
        }
        // History and download records honour the chosen time range; cookies and cache do not.
        let mut recent = 0;
        if kinds.history {
            recent |= COREWEBVIEW2_BROWSING_DATA_KINDS_BROWSING_HISTORY.0;
        }
        if kinds.downloads {
            recent |= COREWEBVIEW2_BROWSING_DATA_KINDS_DOWNLOAD_HISTORY.0;
        }
        let mut jobs = Vec::new();
        if kinds.since_ms > 0 && recent != 0 {
            let since = kinds.since_ms as f64 / 1000.0;
            jobs.push((COREWEBVIEW2_BROWSING_DATA_KINDS(recent), Some(since)));
        } else {
            mask |= recent;
        }
        if mask != 0 {
            jobs.push((COREWEBVIEW2_BROWSING_DATA_KINDS(mask), None));
        }
        if jobs.is_empty() {
            return done(Ok(()));
        }
        match self.any_normal_core() {
            Some(core) => util::clear_profile_data(&core, jobs, done),
            // The profile is only reachable through a view; create a throwaway hidden one.
            None => util::with_temporary_view(&self.shared, move |core| match core {
                Ok(core) => util::clear_profile_data(&core, jobs, done),
                Err(e) => done(Err(e.into())),
            }),
        }
    }

    fn apply_prefs(&mut self, prefs: ProfilePrefs) {
        *self.shared.prefs.borrow_mut() = Some(prefs.clone());
        let cores: Vec<(ICoreWebView2, bool)> = self
            .shared
            .views
            .borrow()
            .values()
            .filter_map(|s| match s {
                Slot::Ready(v) => Some((v.core.clone(), v.state.private)),
                _ => None,
            })
            .collect();
        for (core, private) in cores {
            util::apply_profile_prefs(&core, &prefs.for_view(private));
        }
    }

    fn notify_window_moved(&self) {
        let controllers: Vec<_> = self
            .shared
            .views
            .borrow()
            .values()
            .filter_map(|s| match s {
                Slot::Ready(v) => Some(v.controller.clone()),
                _ => None,
            })
            .collect();
        for c in controllers {
            unsafe {
                let _ = c.NotifyParentWindowPositionChanged();
            }
        }
    }

    fn focus_needs_restoring(&self) -> bool {
        use windows::Win32::UI::Input::KeyboardAndMouse::GetFocus;
        use windows::Win32::UI::WindowsAndMessaging::{GetAncestor, GA_ROOT};
        let focused = unsafe { GetFocus() };
        let root = unsafe { GetAncestor(self.shared.host, GA_ROOT) };
        // A window that closes activates this one while its own webview still has focus; once
        // that webview is gone, focus falls to this frame without another activation event.
        focused.is_invalid() || focused == root || unsafe { GetAncestor(focused, GA_ROOT) } != root
    }

    fn engine_version(&self) -> String {
        let mut v = windows::core::PWSTR::null();
        unsafe {
            if self.shared.env.BrowserVersionString(&mut v).is_ok() {
                return format!("Chromium {} (system WebView2 runtime)", take_pwstr(v));
            }
        }
        "Chromium (system WebView2 runtime)".into()
    }

    fn detach_view(&mut self, tab: TabId) -> Option<Box<dyn std::any::Any>> {
        let slot = self.shared.views.borrow_mut().remove(&tab);
        self.shared.placements.borrow_mut().remove(&tab);
        // Removing a `Creating` slot makes `on_controller_created` close that controller.
        let Some(Slot::Ready(view)) = slot else {
            return None;
        };
        unsafe {
            let _ = view.controller.SetIsVisible(false);
        }
        Some(Box::new(view))
    }

    fn attach_view(&mut self, tab: TabId, view: Box<dyn std::any::Any>) -> AppResult<()> {
        let view = view
            .downcast::<View>()
            .map_err(|_| AppError::Engine("not a WebView2 view".into()))?;
        // Its event handlers keep reporting through the old engine's sink; the core routes
        // events by tab, so they reach this window.
        unsafe { view.controller.SetParentWindow(self.shared.host)? };
        self.shared
            .views
            .borrow_mut()
            .insert(tab, Slot::Ready(*view));
        Ok(())
    }

    fn fill_login(
        &self,
        tab: TabId,
        origin: &str,
        username: &str,
        password: &str,
        done: Box<dyn FnOnce(AppResult<bool>)>,
    ) {
        match (self.view_state(tab), self.core(tab)) {
            (Some(state), Ok(core)) => {
                passwords::fill(&core, &state, origin, username, password, done)
            }
            _ => done(Err(AppError::Engine("view not ready".into()))),
        }
    }
}

fn to_rect(r: Rect) -> RECT {
    RECT {
        left: r.x,
        top: r.y,
        right: r.x + r.width,
        bottom: r.y + r.height,
    }
}

fn deny_popup(shared: &Shared, request: RequestId) {
    // Taken out first: `if let` would keep the borrow alive across the WebView2 call.
    let entry = shared.popups.borrow_mut().remove(&request);
    if let Some((_args, deferral)) = entry {
        // Handled was already set; completing without NewWindow suppresses the popup.
        unsafe {
            let _ = deferral.Complete();
        }
    }
}

impl Drop for WinEngine {
    fn drop(&mut self) {
        let tabs: Vec<TabId> = self.shared.views.borrow().keys().copied().collect();
        for tab in tabs {
            self.destroy_view(tab);
        }
    }
}
