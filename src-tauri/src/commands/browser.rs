//! Tabs, navigation, layout, menus and prompts.

use crate::browser::{with_browser, with_caller};
use crate::errors::{AppError, AppResult};
use crate::platform::TabId;
use crate::shell;
use serde_json::Value;
use tauri::{AppHandle, Webview};

#[tauri::command]
pub fn browser_snapshot(webview: Webview) -> AppResult<Value> {
    let window = crate::browser::window_id(webview.label())
        .ok_or_else(|| AppError::InvalidInput("caller is not a browser window".into()))?;
    with_browser(|b| {
        // Not attached yet while the window's engine starts; the shell retries.
        let state = b
            .initial_state(window)
            .ok_or_else(|| AppError::NotFound("window".into()))?;
        serde_json::to_value(state).map_err(|e| AppError::Engine(e.to_string()))
    })?
}

/// Opens a new browser window (Ctrl+N). Async: creating a webview inside a sync command deadlocks.
#[tauri::command]
pub async fn window_new(app: AppHandle, private: Option<bool>) -> AppResult<()> {
    let seed = match private {
        Some(true) => crate::browser::WindowSeed::Private,
        _ => crate::browser::WindowSeed::New,
    };
    shell::create_window(&app, seed).map(|_| ())
}

#[tauri::command]
pub fn tab_create(
    webview: Webview,
    url: Option<String>,
    private: Option<bool>,
    background: Option<bool>,
) -> AppResult<TabId> {
    use crate::browser::url::{internal_page, web_url};
    if let Some(u) = &url {
        if internal_page(u).is_none() && web_url(u).is_none() {
            return Err(AppError::InvalidInput(
                "tabs open http(s) or internal pages only".into(),
            ));
        }
    }
    with_caller(&webview, |b| {
        let opener = if background.unwrap_or(false) {
            b.active_tab_id()
        } else {
            None
        };
        b.create_tab(
            url,
            private.unwrap_or(false),
            !background.unwrap_or(false),
            opener,
        )
    })
}

#[tauri::command]
pub fn tab_close(webview: Webview, id: TabId) -> AppResult<()> {
    with_caller(&webview, |b| b.close_tab(id))
}

/// `target`: a tab id, or "next" / "previous" / "last" / an index like "#3".
#[tauri::command]
pub fn tab_activate(webview: Webview, id: Option<TabId>, target: Option<String>) -> AppResult<()> {
    with_caller(&webview, |b| match (id, target.as_deref()) {
        (Some(id), _) => b.activate(id),
        (None, Some("next")) => b.activate_relative(true),
        (None, Some("previous")) => b.activate_relative(false),
        (None, Some("last")) => b.activate_index(None),
        (None, Some(t)) => {
            if let Some(i) = t.strip_prefix('#').and_then(|n| n.parse::<usize>().ok()) {
                b.activate_index(Some(i));
            }
        }
        _ => {}
    })
}

#[tauri::command]
pub fn tab_move(webview: Webview, id: TabId, index: usize) -> AppResult<()> {
    with_caller(&webview, |b| b.move_tab(id, index, None))
}

fn caller_window(webview: &Webview) -> AppResult<crate::browser::WindowId> {
    crate::browser::window_id(webview.label())
        .ok_or_else(|| AppError::InvalidInput("caller is not a browser window".into()))
}

/// A drag of tab `id` started in the caller's tab strip (see `Browser::start_tab_drag`).
#[tauri::command]
pub fn tab_drag_start(webview: Webview, id: TabId) -> AppResult<()> {
    let window = caller_window(&webview)?;
    with_browser(|b| b.start_tab_drag(window, id))?
}

/// The dragged tab was dropped on the caller's tab strip, from this window or another one.
/// `group`: dropped onto that group, so the tab joins it.
#[tauri::command]
pub fn tab_drop(webview: Webview, index: usize, group: Option<u64>) -> AppResult<()> {
    let window = caller_window(&webview)?;
    with_browser(|b| b.drop_tab(window, index, group))?
}

/// Moves a tab, page as it is, into a new window at `x`, `y` (screen, logical px). `follow`: the
/// tab is still being dragged and the window follows the pointer until it is dropped. A window's
/// only tab stays put.
#[tauri::command]
pub fn tab_tear_off(
    app: AppHandle,
    webview: Webview,
    id: TabId,
    x: f64,
    y: f64,
    follow: Option<bool>,
) -> AppResult<()> {
    if !x.is_finite() || !y.is_finite() {
        return Err(AppError::InvalidInput("drop position".into()));
    }
    let from = with_caller(&webview, |w| w.can_tear_off(id).then(|| w.id()))?
        .ok_or_else(|| AppError::InvalidInput("a window's only tab cannot move".into()))?;
    let at = (x.clamp(-32_000.0, 32_000.0), y.clamp(-32_000.0, 32_000.0));
    shell::tear_off(&app, from, id, Some(at), follow.unwrap_or(false));
    Ok(())
}

#[tauri::command]
pub fn tab_reopen_closed(webview: Webview) -> AppResult<()> {
    with_caller(&webview, |b| b.reopen_closed())
}

// Menus and the overlay are async on purpose: sync commands run inside WebView2's IPC callback
// on the UI thread, and creating a webview or running a modal menu loop there deadlocks
// (tauri-apps/wry#583). Async commands return to the event loop first. Menu flags come from the
// shell's state because browser state is only reachable on the UI thread.

#[tauri::command]
pub async fn tab_context_menu(
    webview: Webview,
    id: TabId,
    info: shell::TabMenuInfo,
    x: f64,
    y: f64,
) -> AppResult<()> {
    shell::popup_tab_menu(&webview.window(), id, &info, x, y)
}

#[tauri::command]
pub async fn tab_group_menu(
    webview: Webview,
    id: u64,
    collapsed: bool,
    x: f64,
    y: f64,
) -> AppResult<()> {
    shell::popup_group_menu(&webview.window(), id, collapsed, x, y)
}

#[tauri::command]
pub fn tab_group_update(
    webview: Webview,
    group: u64,
    title: Option<String>,
    color: Option<String>,
    collapsed: Option<bool>,
    icon: Option<String>,
) -> AppResult<()> {
    with_caller(&webview, |b| {
        b.group_update(
            group,
            title.as_deref(),
            color.as_deref(),
            collapsed,
            icon.as_deref(),
        )
    })?
}

/// `layout`: columns | rows | mainLeft | mainTop | grid; `count`: 2–4 panes.
#[tauri::command]
pub fn split_set(webview: Webview, count: usize, layout: String) -> AppResult<()> {
    let layout = crate::browser::split::SplitLayout::parse(&layout)
        .ok_or_else(|| AppError::InvalidInput("unknown split layout".into()))?;
    with_caller(&webview, |b| b.set_split(count, layout))
}

#[tauri::command]
pub fn split_close(webview: Webview) -> AppResult<()> {
    with_caller(&webview, |b| b.close_split())
}

#[tauri::command]
pub async fn menu_open_main(webview: Webview, can_reopen: bool, x: f64, y: f64) -> AppResult<()> {
    shell::popup_main_menu(&webview.window(), x, y, can_reopen)
}

#[tauri::command]
pub fn nav_go(webview: Webview, id: TabId, input: String) -> AppResult<()> {
    if input.len() > 32 * 1024 {
        return Err(AppError::InvalidInput("address is too long".into()));
    }
    with_caller(&webview, |b| b.navigate(id, &input))?
}

#[tauri::command]
pub fn nav_back(webview: Webview, id: TabId) -> AppResult<()> {
    with_caller(&webview, |b| b.go_back(id))?
}

#[tauri::command]
pub fn nav_forward(webview: Webview, id: TabId) -> AppResult<()> {
    with_caller(&webview, |b| b.go_forward(id))?
}

#[tauri::command]
pub fn nav_reload(webview: Webview, id: TabId) -> AppResult<()> {
    with_caller(&webview, |b| b.reload(id))?
}

#[tauri::command]
pub fn nav_stop(webview: Webview, id: TabId) -> AppResult<()> {
    with_caller(&webview, |b| b.stop(id))?
}

/// `step`: +1 zoom in, -1 zoom out, 0 reset.
#[tauri::command]
pub fn zoom_change(webview: Webview, id: TabId, step: i32) -> AppResult<()> {
    with_caller(&webview, |b| b.zoom(id, step.signum()))?
}

#[tauri::command]
pub fn find_in_page(webview: Webview, text: String, forward: bool) -> AppResult<()> {
    with_caller(&webview, |b| b.find(&text, forward))?
}

#[tauri::command]
pub fn find_stop(webview: Webview) -> AppResult<()> {
    with_caller(&webview, |b| b.find_stop())
}

#[tauri::command]
pub fn print_page(webview: Webview, id: TabId) -> AppResult<()> {
    with_caller(&webview, |b| b.print(id))?
}

#[tauri::command]
pub fn focus_content(webview: Webview) -> AppResult<()> {
    with_caller(&webview, |b| b.focus_content())
}

#[tauri::command]
pub fn https_continue(webview: Webview, id: TabId) -> AppResult<()> {
    with_caller(&webview, |b| b.https_continue(id))?
}

/// The shell reports its chrome height (tab strip + toolbar + bars) in CSS pixels.
/// Also used for window-level actions the shell triggers.
#[tauri::command]
pub fn layout_update(
    webview: Webview,
    chrome_height: f64,
    action: Option<String>,
) -> AppResult<()> {
    let maximize = with_caller(&webview, |b| {
        b.set_chrome_height(chrome_height);
        if action.as_deref() == Some("toggleFullscreen") {
            b.toggle_window_fullscreen();
        }
        b.take_pending_maximize()
    })?;
    // The window starts hidden to avoid a blank flash; show it once the shell has laid out.
    let w = webview.window();
    if !w.is_visible().unwrap_or(true) {
        w.show()?;
        if maximize {
            w.maximize()?;
        }
        w.set_focus()?;
    }
    Ok(())
}

#[tauri::command]
pub async fn overlay_show(
    webview: Webview,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    focus: bool,
    refocus_shell: Option<bool>,
) -> AppResult<()> {
    let refocus_shell = refocus_shell.unwrap_or(false);
    shell::overlay_show(&webview, x, y, width, height, focus, refocus_shell)
}

#[tauri::command]
pub async fn overlay_hide(webview: Webview, refocus_shell: bool) -> AppResult<()> {
    shell::overlay_hide(&webview, refocus_shell)
}

#[tauri::command]
pub fn permission_respond(webview: Webview, id: u64, allow: bool, remember: bool) -> AppResult<()> {
    with_caller(&webview, |b| b.respond_prompt(id, allow, remember))?
}

#[tauri::command]
pub fn external_protocol_respond(webview: Webview, id: u64, allow: bool) -> AppResult<()> {
    with_caller(&webview, |b| b.respond_prompt(id, allow, false))?
}
