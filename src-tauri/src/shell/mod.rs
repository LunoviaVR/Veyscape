//! Hosting for the trusted browser UI: browser windows, each with a React "shell" webview and a
//! popup "overlay" webview, and native menus. Z-order inside a window (bottom → top):
//! shell webview (full window) → content host HWND (web pages) → overlay webview (popups).

#[cfg(windows)]
mod follow;

use crate::browser::{self, WindowId, WindowSeed};
use crate::errors::{AppError, AppResult};
use crate::state::Services;
use crate::storage::Bookmark;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tauri::menu::{
    IconMenuItemBuilder, IsMenuItem, Menu, MenuBuilder, MenuEvent, MenuItemBuilder,
    PredefinedMenuItem, Submenu, SubmenuBuilder,
};
use tauri::webview::WebviewBuilder;
use tauri::window::WindowBuilder;
use tauri::{
    AppHandle, Emitter, EventTarget, LogicalPosition, LogicalSize, Manager, PhysicalPosition,
    Theme, WebviewUrl,
};

/// Opens a browser window with its shell webview. The window's engine starts once the shell's
/// WebView2 environment exists (tabs share that environment, in their own profile), so it
/// becomes usable shortly after this returns. Must not run inside a sync command: creating a
/// webview there deadlocks (tauri-apps/wry#583).
pub fn create_window(app: &AppHandle, seed: WindowSeed) -> AppResult<WindowId> {
    let id = browser::next_window_id();
    // Windows places each new window (CW_USEDEFAULT), cascading it from the previous one.
    let window = WindowBuilder::new(app, browser::window_label(id))
        .title("Veyscape")
        .inner_size(1280.0, 820.0)
        .min_inner_size(520.0, 360.0)
        // The tab strip doubles as the title bar (saves ~32px of vertical space).
        .decorations(false)
        .shadow(true)
        .visible(false)
        .build()?;
    match &seed {
        WindowSeed::Restore(saved) => place_restored(&window, saved.bounds)?,
        WindowSeed::Adopt { from, at, .. } => {
            // Same size as the window the tab came from, its first tab under the pointer.
            if let Some(source) = app.get_window(&browser::window_label(*from)) {
                window.set_size(source.inner_size()?)?;
            }
            if let Some((x, y)) = at {
                window.set_position(tauri::LogicalPosition::new(x - GRAB.0, y - GRAB.1))?;
            }
        }
        _ => {}
    }
    let dark = window.theme().map(|t| t == Theme::Dark).unwrap_or(false);
    let background = if dark {
        tauri::window::Color(24, 24, 27, 255)
    } else {
        tauri::window::Color(250, 250, 250, 255)
    };
    let size = window.inner_size()?;
    let shell = window.add_child(
        WebviewBuilder::new(
            browser::shell_label(id),
            WebviewUrl::App("index.html".into()),
        )
        .background_color(background)
        .disable_drag_drop_handler()
        .zoom_hotkeys_enabled(false)
        .auto_resize(),
        PhysicalPosition::new(0, 0),
        size,
    )?;
    window.on_window_event(move |event| on_window_event(id, event));

    let app2 = app.clone();
    shell.with_webview(move |platform| {
        if let Err(e) = start_window(&app2, id, seed, platform) {
            log::error!("browser window {id} failed to start: {e}");
        }
    })?;
    Ok(id)
}

/// Where a torn-off window is held, in CSS px from its top-left corner: over its first tab.
const GRAB: (f64, f64) = (110.0, 18.0);

/// Opens a new window that takes over tab `tab` of window `from` (see `WindowSeed::Adopt`).
/// `follow`: the tab is still being dragged; the window follows the pointer until the release.
/// UI thread only (sync commands, menu handlers). The window itself is created on another
/// thread, because creating one inside a WebView2 callback deadlocks (tauri-apps/wry#583).
pub fn tear_off(app: &AppHandle, from: WindowId, tab: u64, at: Option<(f64, f64)>, follow: bool) {
    browser::begin_tear_off(tab);
    #[cfg(not(windows))]
    let _ = follow; // no pointer-follow on other platforms yet
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        match create_window(&app, WindowSeed::Adopt { from, tab, at }) {
            #[cfg(windows)]
            Ok(id) if follow => {
                tauri::async_runtime::spawn_blocking(move || {
                    follow::follow_pointer(&app, id, GRAB)
                });
            }
            Ok(_) => {}
            Err(e) => log::warn!("opening a window for the moved tab failed: {e}"),
        }
    });
}

/// Puts a restored window where it was, unless that spot is no longer on any monitor.
fn place_restored(
    window: &tauri::Window,
    bounds: Option<browser::session::WindowBounds>,
) -> AppResult<()> {
    let Some(b) = bounds else { return Ok(()) };
    let monitors: Vec<(i32, i32, u32, u32)> = window
        .available_monitors()?
        .iter()
        .map(|m| {
            (
                m.position().x,
                m.position().y,
                m.size().width,
                m.size().height,
            )
        })
        .collect();
    if b.visible_on(&monitors) {
        window.set_position(PhysicalPosition::new(b.x, b.y))?;
        window.set_size(tauri::PhysicalSize::new(b.width, b.height))?;
    }
    Ok(())
}

fn on_window_event(id: WindowId, event: &tauri::WindowEvent) {
    use tauri::WindowEvent as W;
    match event {
        W::Resized(_) | W::ScaleFactorChanged { .. } => {
            let _ = browser::with_window(id, |w| {
                w.relayout();
                w.bounds_changed();
            });
        }
        W::Moved(_) => {
            let _ = browser::with_window(id, |w| {
                w.notify_window_moved();
                w.bounds_changed();
            });
        }
        // Child webviews do not receive focus when the window is activated; hand it on.
        W::Focused(true) => {
            let _ = browser::with_window(id, |w| w.restore_focus());
        }
        // The browser decides: the last window quits (and stays in the session).
        W::CloseRequested { api, .. } => {
            api.prevent_close();
            let _ = browser::with_browser(|b| b.request_close(id));
        }
        _ => {}
    }
}

fn engine_hooks(app: &AppHandle, id: WindowId) -> crate::platform::EngineHooks {
    let services = app.state::<Arc<Services>>();
    crate::platform::EngineHooks {
        sink: std::rc::Rc::new(move |event| browser::dispatch(id, event)),
        filter: services.adblock.clone(),
        settings: services.settings_cell.clone(),
    }
}

#[cfg(windows)]
fn start_window(
    app: &AppHandle,
    id: WindowId,
    seed: WindowSeed,
    platform: tauri::webview::PlatformWebview,
) -> AppResult<()> {
    let window = app
        .get_window(&browser::window_label(id))
        .ok_or_else(|| AppError::NotFound("browser window".into()))?;
    if let Ok(core) = unsafe { platform.controller().CoreWebView2() } {
        crate::platform::restrict_ui_context_menu(&core)?;
    }
    let engine = crate::platform::PlatformEngine::new(
        &window,
        platform.environment(),
        engine_hooks(app, id),
    )?;
    browser::with_browser(|b| b.attach_window(id, Box::new(engine), seed))
}

#[cfg(not(windows))]
fn start_window(
    app: &AppHandle,
    id: WindowId,
    seed: WindowSeed,
    _platform: tauri::webview::PlatformWebview,
) -> AppResult<()> {
    let window = app
        .get_window(&browser::window_label(id))
        .ok_or_else(|| AppError::NotFound("browser window".into()))?;
    let engine = crate::platform::PlatformEngine::new(&window, engine_hooks(app, id))?;
    browser::with_browser(|b| b.attach_window(id, Box::new(engine), seed))
}

fn caller_window(caller: &tauri::Webview) -> AppResult<WindowId> {
    browser::window_id(caller.label())
        .ok_or_else(|| AppError::InvalidInput("caller is not a browser window".into()))
}

/// Shows the caller's window's popup overlay at a rectangle in shell CSS pixels, creating it on
/// first use.
pub fn overlay_show(
    caller: &tauri::Webview,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    focus: bool,
    refocus_shell: bool,
) -> AppResult<()> {
    let label = browser::overlay_label(caller_window(caller)?);
    let window = caller.window();
    // Async commands run concurrently; serialize so each overlay is created exactly once.
    static CREATE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = CREATE.lock().unwrap_or_else(|p| p.into_inner());
    let position = LogicalPosition::new(x.max(0.0), y.max(0.0));
    let size = LogicalSize::new(width.clamp(1.0, 4000.0), height.clamp(1.0, 4000.0));
    let overlay = match caller.app_handle().get_webview(&label) {
        Some(o) => {
            o.set_position(position)?;
            o.set_size(size)?;
            o.show()?;
            o
        }
        // Created after the content host, so it stacks above web content.
        None => {
            let o = window.add_child(
                WebviewBuilder::new(&label, WebviewUrl::App("index.html?view=overlay".into()))
                    .transparent(true)
                    // Never steal focus from the address bar while suggestions appear.
                    .focused(false)
                    .disable_drag_drop_handler()
                    .zoom_hotkeys_enabled(false),
                position,
                size,
            )?;
            // The window's content host keeps itself directly beneath this webview from now on.
            #[cfg(windows)]
            o.with_webview(|pw| unsafe {
                let mut hwnd = windows::Win32::Foundation::HWND::default();
                if pw.controller().ParentWindow(&mut hwnd).is_ok() {
                    crate::platform::set_overlay_window(hwnd.0 as isize);
                }
                if let Ok(core) = pw.controller().CoreWebView2() {
                    let _ = crate::platform::restrict_ui_context_menu(&core);
                }
            })?;
            o
        }
    };
    if focus {
        overlay.set_focus()?;
    } else if refocus_shell {
        // A click in the overlay (e.g. removing a suggestion) gave it focus; typing goes on in the shell.
        if let Some(shell) = caller
            .app_handle()
            .get_webview(&browser::shell_label(caller_window(caller)?))
        {
            shell.set_focus()?;
        }
    }
    Ok(())
}

/// Hides the caller's window's overlay; optionally gives keyboard focus back to its shell.
pub fn overlay_hide(caller: &tauri::Webview, refocus_shell: bool) -> AppResult<()> {
    let id = caller_window(caller)?;
    let app = caller.app_handle();
    if let Some(o) = app.get_webview(&browser::overlay_label(id)) {
        o.hide()?;
    }
    if refocus_shell {
        if let Some(s) = app.get_webview(&browser::shell_label(id)) {
            s.set_focus()?;
        }
    }
    Ok(())
}

/// Native menus are used for the main menu and tab context menu: they are accessible to screen
/// readers and keyboard users out of the box and draw above web content without an overlay.
pub fn popup_main_menu(win: &tauri::Window, x: f64, y: f64, can_reopen: bool) -> AppResult<()> {
    let item = |id: &str, text: &str, accel: Option<&str>| {
        let mut b = MenuItemBuilder::with_id(id, text);
        if let Some(a) = accel {
            b = b.accelerator(a);
        }
        b.build(win)
    };
    let menu = MenuBuilder::new(win)
        .item(&item("action:newTab", "New tab", Some("Ctrl+T"))?)
        .item(&item("action:newWindow", "New window", Some("Ctrl+N"))?)
        .item(&item(
            "action:newPrivateTab",
            "New private tab",
            Some("Ctrl+Shift+N"),
        )?)
        .item(&item(
            "action:newPrivateWindow",
            "New private window",
            Some("Ctrl+Shift+P"),
        )?)
        .item(
            &MenuItemBuilder::with_id("action:reopenClosedTab", "Reopen closed tab")
                .accelerator("Ctrl+Shift+T")
                .enabled(can_reopen)
                .build(win)?,
        )
        .item(&PredefinedMenuItem::separator(win)?)
        .item(&item("action:openBookmarks", "Bookmarks", None)?)
        .item(&item(
            "action:toggleBookmarkBar",
            "Show or hide bookmarks bar",
            Some("Ctrl+Shift+B"),
        )?)
        .item(&item("action:openHistory", "History", Some("Ctrl+H"))?)
        .item(&item("action:openDownloads", "Downloads", Some("Ctrl+J"))?)
        .item(&PredefinedMenuItem::separator(win)?)
        .item(&item("action:findInPage", "Find in page…", Some("Ctrl+F"))?)
        .item(&item("action:zoomIn", "Zoom in", Some("Ctrl+="))?)
        .item(&item("action:zoomOut", "Zoom out", Some("Ctrl+-"))?)
        .item(&item("action:zoomReset", "Reset zoom", Some("Ctrl+0"))?)
        .item(&item("action:print", "Print…", Some("Ctrl+P"))?)
        .item(&item(
            "action:toggleFullscreen",
            "Full screen",
            Some("F11"),
        )?)
        .item(&PredefinedMenuItem::separator(win)?)
        .item(&item("action:openSettings", "Settings", None)?)
        .item(&item("action:openAbout", "About Veyscape", None)?)
        .item(&PredefinedMenuItem::separator(win)?)
        .item(&item("action:quit", "Exit", None)?)
        .build()?;
    popup(win, &menu, x, y)
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupRef {
    pub id: u64,
    pub title: String,
}

/// What the shell knows about the tab under the cursor (browser state lives on the UI thread).
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TabMenuInfo {
    pub pinned: bool,
    pub can_reopen: bool,
    pub group_id: Option<u64>,
    pub groups: Vec<GroupRef>,
    pub in_split: bool,
    /// The window's only tab (it cannot move to a new window).
    #[serde(default)]
    pub only_tab: bool,
}

pub fn popup_tab_menu(
    win: &tauri::Window,
    tab: u64,
    info: &TabMenuInfo,
    x: f64,
    y: f64,
) -> AppResult<()> {
    let mut menu = MenuBuilder::new(win)
        .text(format!("tab:reload:{tab}"), "Reload")
        .text(format!("tab:duplicate:{tab}"), "Duplicate")
        .text(
            format!("tab:pin:{tab}"),
            if info.pinned { "Unpin" } else { "Pin" },
        )
        .item(
            &MenuItemBuilder::with_id(format!("tab:moveToNewWindow:{tab}"), "Move to new window")
                .enabled(!info.only_tab)
                .build(win)?,
        )
        .separator();
    if !info.pinned {
        menu = menu.text(format!("tab:newGroup:{tab}"), "Add to new group");
        let others: Vec<&GroupRef> = info
            .groups
            .iter()
            .filter(|g| Some(g.id) != info.group_id)
            .collect();
        if !others.is_empty() {
            let mut sub = SubmenuBuilder::new(win, "Add to group");
            for (i, g) in others.iter().enumerate() {
                let label = menu_label(&g.title, &format!("Group {}", i + 1));
                sub = sub.text(format!("tab:addToGroup:{tab}:{}", g.id), label);
            }
            menu = menu.item(&sub.build()?);
        }
        if info.group_id.is_some() {
            menu = menu.text(format!("tab:removeFromGroup:{tab}"), "Remove from group");
        }
        let split_label = if info.in_split {
            "Already in split view"
        } else {
            "Open in split view"
        };
        menu = menu
            .item(
                &MenuItemBuilder::with_id(format!("tab:split:{tab}"), split_label)
                    .enabled(!info.in_split)
                    .build(win)?,
            )
            .separator();
    }
    let menu = menu
        .text(format!("tab:close:{tab}"), "Close tab")
        .text(format!("tab:closeOthers:{tab}"), "Close other tabs")
        .item(
            &MenuItemBuilder::with_id("action:reopenClosedTab", "Reopen closed tab")
                .enabled(info.can_reopen)
                .build(win)?,
        )
        .build()?;
    popup(win, &menu, x, y)
}

/// Group chip menu: edit (name, icon, colour), collapse, ungroup, close.
pub fn popup_group_menu(
    win: &tauri::Window,
    group: u64,
    collapsed: bool,
    x: f64,
    y: f64,
) -> AppResult<()> {
    let menu = MenuBuilder::new(win)
        .text(format!("group:edit:{group}"), "Edit group…")
        .text(
            format!("group:collapse:{group}"),
            if collapsed { "Expand" } else { "Minimize" },
        )
        .separator()
        .text(format!("group:ungroup:{group}"), "Ungroup")
        .text(format!("group:close:{group}"), "Close group")
        .build()?;
    popup(win, &menu, x, y)
}

fn on_group_menu(app: &AppHandle, rest: &str) {
    let mut parts = rest.splitn(3, ':');
    let (Some(op), Some(Ok(id))) = (parts.next(), parts.next().map(str::parse::<u64>)) else {
        return;
    };
    match op {
        // The shell shows the editor (name, icon, colour) in the overlay, above web content.
        "edit" => emit_to_menu_window(app, "group:editor", id),
        _ => {
            let _ = with_menu_window(|b| match op {
                "collapse" => {
                    let collapsed = b
                        .snapshot()
                        .groups
                        .iter()
                        .any(|g| g.id == id && g.collapsed);
                    let _ = b.group_update(id, None, None, Some(!collapsed), None);
                }
                "ungroup" => b.group_ungroup(id),
                "close" => b.group_close(id),
                _ => {}
            });
        }
    }
}

/// The window that showed the last popup menu. Menu events carry no window, and only one popup
/// menu can be open at a time, so its clicks belong to this window.
static MENU_WINDOW: AtomicU32 = AtomicU32::new(0);

fn popup(win: &tauri::Window, menu: &Menu<tauri::Wry>, x: f64, y: f64) -> AppResult<()> {
    if let Some(id) = browser::window_id(win.label()) {
        MENU_WINDOW.store(id, Ordering::Relaxed);
    }
    win.popup_menu_at(menu, LogicalPosition::new(x, y))?;
    Ok(())
}

/// Runs `f` on the window whose menu was clicked.
fn with_menu_window<T>(f: impl FnOnce(&mut browser::Window) -> T) -> AppResult<T> {
    browser::with_window(MENU_WINDOW.load(Ordering::Relaxed), f)
}

/// Sends an event to the shell of the window whose menu was clicked.
fn emit_to_menu_window<S: serde::Serialize + Clone>(app: &AppHandle, event: &str, payload: S) {
    let shell = browser::shell_label(MENU_WINDOW.load(Ordering::Relaxed));
    let _ = app.emit_to(EventTarget::webview(shell), event, payload);
}

/// Bookmarks changed: every window refreshes its bar, pages and star buttons.
fn bookmarks_changed(app: &AppHandle) {
    let _ = app.emit("bookmarks:changed", ());
    let _ = browser::with_browser(|b| b.refresh_bookmarks());
}

/// Menu items either name a shell action (`action:<name>`) — executed by the shell so it
/// behaves exactly like the keyboard shortcut — or a tab operation (`tab:<op>:<id>`).
/// Menu label: `&` is a mnemonic marker on Windows, so escape it; keep labels short.
fn menu_label(title: &str, fallback: &str) -> String {
    let t = if title.trim().is_empty() {
        fallback
    } else {
        title.trim()
    };
    let short: String = t.chars().take(60).collect();
    short.replace('&', "&&")
}

fn bookmark_children(
    win: &tauri::Window,
    all: &[Bookmark],
    parent: i64,
    depth: usize,
) -> AppResult<Vec<Box<dyn IsMenuItem<tauri::Wry>>>> {
    let mut items: Vec<Box<dyn IsMenuItem<tauri::Wry>>> = Vec::new();
    // Menus draw item bitmaps at their pixel size: 16 logical px on this display.
    let icon_px = (16.0 * win.scale_factor().unwrap_or(1.0)).round() as u32;
    // Folder 0 stands for the bar itself (root items), used by the bar's overflow button.
    let wanted = (parent != 0).then_some(parent);
    for b in all.iter().filter(|b| b.parent_id == wanted) {
        if b.kind == "folder" {
            let mut sub = SubmenuBuilder::new(win, menu_label(&b.title, "Folder"));
            let children = if depth < 8 {
                bookmark_children(win, all, b.id, depth + 1)?
            } else {
                Vec::new()
            };
            if children.is_empty() {
                sub = sub.item(&MenuItemBuilder::new("(empty)").enabled(false).build(win)?);
            }
            for c in &children {
                sub = sub.item(c.as_ref());
            }
            let built: Submenu<tauri::Wry> = sub.build()?;
            items.push(Box::new(built));
        } else {
            let label = menu_label(&b.title, b.url.as_deref().unwrap_or("Bookmark"));
            let id = format!("bm:open:{}", b.id);
            let icon = b
                .favicon
                .as_deref()
                .and_then(|f| crate::bookmarks::favicon::rgba(f, icon_px));
            let item: Box<dyn IsMenuItem<tauri::Wry>> = match icon {
                Some(i) => Box::new(
                    IconMenuItemBuilder::with_id(id, label)
                        .icon(tauri::image::Image::new_owned(i.pixels, i.width, i.height))
                        .build(win)?,
                ),
                None => Box::new(MenuItemBuilder::with_id(id, label).build(win)?),
            };
            items.push(item);
        }
    }
    Ok(items)
}

/// Dropdown for a folder on the bookmark bar; nested folders become submenus.
pub fn popup_bookmark_folder(
    win: &tauri::Window,
    all: &[Bookmark],
    folder: i64,
    x: f64,
    y: f64,
) -> AppResult<()> {
    let children = bookmark_children(win, all, folder, 0)?;
    let mut menu = MenuBuilder::new(win);
    if children.is_empty() {
        menu = menu.item(&MenuItemBuilder::new("(empty)").enabled(false).build(win)?);
    }
    for c in &children {
        menu = menu.item(c.as_ref());
    }
    popup(win, &menu.build()?, x, y)
}

/// Right-click menu for a bookmark bar item.
pub fn popup_bookmark_item(win: &tauri::Window, item: &Bookmark, x: f64, y: f64) -> AppResult<()> {
    let id = item.id;
    let mut menu = MenuBuilder::new(win);
    if item.kind == "bookmark" {
        menu = menu
            .text(format!("bm:open:{id}"), "Open")
            .text(format!("bm:newtab:{id}"), "Open in new tab")
            .text(format!("bm:private:{id}"), "Open in private tab")
            .separator();
    }
    let folder = item.kind == "folder";
    let mut menu = menu
        .text(
            format!("bm:edit:{id}"),
            if folder {
                "Edit folder…"
            } else {
                "Edit bookmark…"
            },
        )
        .text(format!("bm:delete:{id}"), "Delete")
        .separator();
    // Inside a folder, new items go into that folder; otherwise onto the bar.
    let target = if folder {
        id.to_string()
    } else {
        "bar".to_string()
    };
    menu = menu
        .text(
            format!("bmnew:bookmark:{target}"),
            if folder {
                "Add bookmark to this folder…"
            } else {
                "Add bookmark…"
            },
        )
        .text(
            format!("bmnew:folder:{target}"),
            if folder {
                "New folder inside…"
            } else {
                "New folder…"
            },
        )
        .separator()
        .text("action:toggleBookmarkBar", "Hide bookmarks bar");
    popup(win, &menu.build()?, x, y)
}

/// Right-click on empty space in the bookmarks bar.
pub fn popup_bookmark_bar_menu(
    win: &tauri::Window,
    can_bookmark_page: bool,
    x: f64,
    y: f64,
) -> AppResult<()> {
    let menu = MenuBuilder::new(win)
        .item(
            &MenuItemBuilder::with_id("bmbar:addPage", "Bookmark this page")
                .accelerator("Ctrl+D")
                .enabled(can_bookmark_page)
                .build(win)?,
        )
        .text("bmnew:bookmark:bar", "Add bookmark…")
        .text("bmnew:folder:bar", "New folder…")
        .separator()
        .text("action:openBookmarks", "Manage bookmarks")
        .text("action:toggleBookmarkBar", "Hide bookmarks bar")
        .build()?;
    popup(win, &menu, x, y)
}

/// Asks the shell to open the bookmark/folder editor (drawn in the overlay above web content).
fn open_bookmark_editor(app: &AppHandle, mode: &str, id: Option<i64>, parent: Option<i64>) {
    #[derive(serde::Serialize, Clone)]
    #[serde(rename_all = "camelCase")]
    struct EditorRequest<'a> {
        mode: &'a str,
        id: Option<i64>,
        parent_id: Option<i64>,
    }
    emit_to_menu_window(
        app,
        "bookmark:editor",
        EditorRequest {
            mode,
            id,
            parent_id: parent,
        },
    );
}

fn on_bookmark_menu(app: &AppHandle, op: &str, id: i64) {
    let services = app.state::<Arc<Services>>();
    if op == "delete" {
        if services.db.bookmark_remove(id).is_ok() {
            bookmarks_changed(app);
        }
        return;
    }
    if op == "edit" {
        if let Ok(b) = services.db.bookmark_get(id) {
            open_bookmark_editor(
                app,
                if b.kind == "folder" {
                    "folder"
                } else {
                    "bookmark"
                },
                Some(id),
                b.parent_id,
            );
        }
        return;
    }
    let Some(url) = services.db.bookmark_get(id).ok().and_then(|b| b.url) else {
        return;
    };
    let _ = with_menu_window(|b| match op {
        "open" => {
            if let Some(tab) = b.active_tab_id() {
                let _ = b.navigate(tab, &url);
            }
        }
        "newtab" => {
            let opener = b.active_tab_id();
            b.create_tab(Some(url), false, false, opener);
        }
        "private" => {
            b.create_tab(Some(url), true, true, None);
        }
        _ => {}
    });
}

pub fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    let id = event.id().as_ref();
    if let Some(rest) = id.strip_prefix("group:") {
        return on_group_menu(app, rest);
    }
    if let Some(rest) = id.strip_prefix("tab:addToGroup:") {
        if let Some((Ok(tab), Ok(group))) = rest
            .split_once(':')
            .map(|(t, g)| (t.parse::<u64>(), g.parse::<u64>()))
        {
            let _ = with_menu_window(|b| b.group_assign(tab, Some(group)));
        }
        return;
    }
    if id == "bmbar:addPage" {
        let saved = with_menu_window(|b| b.bookmark_active_page());
        if matches!(saved, Ok(Ok(true))) {
            bookmarks_changed(app);
        }
        return;
    }
    if let Some(rest) = id.strip_prefix("bmnew:") {
        if let Some((mode, target)) = rest.split_once(':') {
            open_bookmark_editor(app, mode, None, target.parse::<i64>().ok());
        }
        return;
    }
    if let Some(rest) = id.strip_prefix("bm:") {
        if let Some((op, Ok(bid))) = rest.split_once(':').map(|(o, i)| (o, i.parse::<i64>())) {
            on_bookmark_menu(app, op, bid);
        }
        return;
    }
    if let Some(action) = id.strip_prefix("action:") {
        if action == "quit" {
            if browser::with_browser(|b| b.quit()).is_err() {
                app.exit(0);
            }
            return;
        }
        emit_to_menu_window(app, "shell:action", action);
        return;
    }
    let mut parts = id.splitn(3, ':');
    let (Some("tab"), Some(op), Some(Ok(tab))) = (
        parts.next(),
        parts.next(),
        parts.next().map(str::parse::<u64>),
    ) else {
        return;
    };
    if op == "moveToNewWindow" {
        if let Ok(Some(from)) = with_menu_window(|b| b.can_tear_off(tab).then(|| b.id())) {
            tear_off(app, from, tab, None, false);
        }
        return;
    }
    if op == "newGroup" {
        // A new group opens its editor so it can be named right away.
        if let Ok(Ok(group)) = with_menu_window(|b| b.group_create(tab)) {
            emit_to_menu_window(app, "group:editor", group);
        }
        return;
    }
    let _ = with_menu_window(|b| match op {
        "reload" => {
            let _ = b.reload(tab);
        }
        "duplicate" => b.duplicate(tab),
        "pin" => {
            let pinned = b.tab(tab).is_some_and(|t| t.pinned);
            b.set_pinned(tab, !pinned);
        }
        "close" => b.close_tab(tab),
        "removeFromGroup" => b.group_assign(tab, None),
        "split" => b.add_to_split(tab),
        "closeOthers" => b.close_others(tab),
        _ => {}
    });
}
