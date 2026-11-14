//! WebView2 event handlers for one content view. Each handler translates a native event into
//! an [`EngineEvent`], or makes a synchronous decision (request blocking, navigation policy).

use super::util::{get_string, png_data_url, read_stream, strip_edge_brands};
use super::{Shared, ViewState};
use crate::adblock::ResourceKind;
use crate::browser::shortcuts;
use crate::browser::url::{
    https_only_step, https_upgrade, is_forbidden_scheme, is_search_preview_player,
    navigation_policy, origin_of, HttpsOnlyStep, NavigationPolicy,
};
use crate::errors::AppResult;
use crate::permissions::PermissionKind;
use crate::platform::{DownloadState, EngineEvent, TabId};
use std::rc::Rc;
use std::time::{Duration, Instant};
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::*;
use windows::core::{Interface, BOOL, HSTRING};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyState, VK_CONTROL, VK_MBUTTON, VK_MENU, VK_SHIFT,
};

type Token = i64;

fn token() -> Token {
    0
}

fn bool_of(f: impl FnOnce(*mut BOOL) -> windows::core::Result<()>) -> bool {
    let mut b = BOOL::default();
    f(&mut b).is_ok() && b.as_bool()
}

pub fn register(
    shared: &Rc<Shared>,
    controller: &ICoreWebView2Controller,
    core: &ICoreWebView2,
    state: &Rc<ViewState>,
) -> AppResult<()> {
    let tab = state.tab;
    let mut t = token();
    unsafe {
        // --- request filtering (network level, before the request is sent) ---
        match core.cast::<ICoreWebView2_22>() {
            Ok(c22) => c22.AddWebResourceRequestedFilterWithRequestSourceKinds(
                &HSTRING::from("*"),
                COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
                COREWEBVIEW2_WEB_RESOURCE_REQUEST_SOURCE_KINDS_ALL,
            )?,
            Err(_) => core.AddWebResourceRequestedFilter(
                &HSTRING::from("*"),
                COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
            )?,
        }
        let (s, st) = (shared.clone(), state.clone());
        core.add_WebResourceRequested(
            &WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let request = args.Request()?;
                if let Ok(headers) = request.Headers() {
                    hide_edge_client_hints(&headers);
                }
                let uri = get_string(|p| request.Uri(p));
                let mut ctx = COREWEBVIEW2_WEB_RESOURCE_CONTEXT::default();
                args.ResourceContext(&mut ctx)?;
                // The top-level document itself is governed by navigation policy, not filters.
                if ctx == COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT && uri == *st.top_url.borrow()
                {
                    return Ok(());
                }
                // Subresources belong to the shown document, not to a navigation still pending.
                let page = match st.page_url.borrow().as_str() {
                    "" => st.top_url.borrow().clone(),
                    committed => committed.to_string(),
                };
                // Stalls scrolling for ~450 ms in every Chromium browser; see is_search_preview_player.
                if is_search_preview_player(&page, &uri) {
                    let empty = s.env.CreateWebResourceResponse(
                        None,
                        204,
                        &HSTRING::from("No Content"),
                        &HSTRING::new(),
                    )?;
                    args.SetResponse(&empty)?;
                    return Ok(());
                }
                if s.hooks
                    .filter
                    .should_block(&uri, &page, resource_kind(ctx), st.private)
                {
                    let response = s.env.CreateWebResourceResponse(
                        None,
                        403,
                        &HSTRING::from("Blocked"),
                        &HSTRING::new(),
                    )?;
                    args.SetResponse(&response)?;
                    s.emit(EngineEvent::RequestBlocked { tab });
                }
                Ok(())
            })),
            &mut t,
        )?;

        // --- navigation lifecycle ---
        let (s, st, c) = (shared.clone(), state.clone(), core.clone());
        core.add_NavigationStarting(
            &NavigationStartingEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let uri = get_string(|p| args.Uri(p));
                if let NavigationPolicy::Block(reason) = navigation_policy(&uri) {
                    log::info!("blocked navigation in tab {tab}: {reason}");
                    args.SetCancel(true)?;
                    return Ok(());
                }
                let mut nav_id = 0u64;
                args.NavigationId(&mut nav_id)?;
                let https_only = s
                    .hooks
                    .settings
                    .read()
                    .map(|v| v.https_only)
                    .unwrap_or(false);
                if https_only {
                    let upgrading = matches!(&*st.upgrade.borrow(), Some((id, _)) if *id == nav_id);
                    match https_only_step(&uri, upgrading, &st.http_allowed.borrow()) {
                        HttpsOnlyStep::Continue => {}
                        HttpsOnlyStep::Upgrade(upgraded) => {
                            args.SetCancel(true)?;
                            *st.upgrade.borrow_mut() = Some((0, uri));
                            c.Navigate(&HSTRING::from(upgraded))?;
                            return Ok(());
                        }
                        HttpsOnlyStep::Failed => {
                            args.SetCancel(true)?;
                            st.upgrade.take();
                            s.emit(EngineEvent::HttpsUpgradeFailed { tab, http_url: uri });
                            return Ok(());
                        }
                    }
                }
                // Bind a pending upgrade to the navigation that carries the https:// URL.
                if let Some((id, from)) = st.upgrade.borrow_mut().as_mut() {
                    if *id == 0 && https_upgrade(from).as_deref() == Some(uri.as_str()) {
                        *id = nav_id;
                    } else if *id != nav_id {
                        *id = u64::MAX;
                    }
                }
                *st.top_url.borrow_mut() = uri.clone();
                st.top_nav.set(nav_id);
                st.download_started.set(false);
                s.emit(EngineEvent::NavigationStarting { tab, url: uri });
                Ok(())
            })),
            &mut t,
        )?;

        let (s, st) = (shared.clone(), state.clone());
        core.add_NavigationCompleted(
            &NavigationCompletedEventHandler::create(Box::new(move |sender, args| {
                let Some(args) = args else { return Ok(()) };
                let success = bool_of(|b| args.IsSuccess(b));
                let committed = sender
                    .map(|c| get_string(|p| c.Source(p)))
                    .unwrap_or_default();
                let mut nav_id = 0u64;
                args.NavigationId(&mut nav_id)?;
                // Only the navigation bound to the upgrade settles it (the cancelled http one also completes).
                let settled = {
                    let mut up = st.upgrade.borrow_mut();
                    match up.as_ref() {
                        Some((id, _)) if *id == nav_id || *id == u64::MAX => up.take(),
                        _ => None,
                    }
                };
                let mut status = COREWEBVIEW2_WEB_ERROR_STATUS::default();
                args.WebErrorStatus(&mut status)?;
                // A navigation that turned into a download "fails" without being an error.
                let became_download = st.download_started.replace(false);
                let mut reported = false;
                if let Some((id, http_url)) = settled {
                    if id == nav_id
                        && !success
                        && status != COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED
                    {
                        s.emit(EngineEvent::HttpsUpgradeFailed { tab, http_url });
                        reported = true;
                    }
                }
                if !success && !reported && !became_download {
                    if let Some(reason) = load_failure_reason(status) {
                        let url = st.top_url.borrow().clone();
                        s.emit(EngineEvent::LoadFailed { tab, url, reason });
                    }
                }
                // Nothing committed (a download, a 204, Stop): requests come from the shown page
                // again. Only the latest navigation may do this, not a late cancelled older one.
                if !success && nav_id == st.top_nav.get() && !committed.is_empty() {
                    *st.top_url.borrow_mut() = committed.clone();
                    *st.page_url.borrow_mut() = committed.clone();
                }
                s.emit(EngineEvent::NavigationCompleted {
                    tab,
                    success,
                    committed_url: committed,
                });
                Ok(())
            })),
            &mut t,
        )?;

        let (s, st) = (shared.clone(), state.clone());
        core.add_SourceChanged(
            &SourceChangedEventHandler::create(Box::new(move |sender, _| {
                if let Some(core) = sender {
                    let url = get_string(|p| core.Source(p));
                    *st.page_url.borrow_mut() = url.clone();
                    s.emit(EngineEvent::SourceChanged { tab, url });
                }
                Ok(())
            })),
            &mut t,
        )?;

        let s = shared.clone();
        core.add_DocumentTitleChanged(
            &DocumentTitleChangedEventHandler::create(Box::new(move |sender, _| {
                if let Some(core) = sender {
                    s.emit(EngineEvent::TitleChanged {
                        tab,
                        title: get_string(|p| core.DocumentTitle(p)),
                    });
                }
                Ok(())
            })),
            &mut t,
        )?;

        let s = shared.clone();
        core.add_HistoryChanged(
            &HistoryChangedEventHandler::create(Box::new(move |sender, _| {
                if let Some(core) = sender {
                    s.emit(EngineEvent::HistoryChanged {
                        tab,
                        can_go_back: bool_of(|b| core.CanGoBack(b)),
                        can_go_forward: bool_of(|b| core.CanGoForward(b)),
                    });
                }
                Ok(())
            })),
            &mut t,
        )?;

        // --- cosmetic filtering: site-specific CSS early, generic class/id CSS after DOM load ---
        let (s, st) = (shared.clone(), state.clone());
        core.add_ContentLoading(
            &ContentLoadingEventHandler::create(Box::new(move |sender, _| {
                let Some(core) = sender else { return Ok(()) };
                // The new document has committed: it is now the page its requests come from.
                let page = get_string(|p| core.Source(p));
                *st.page_url.borrow_mut() = page.clone();
                st.generic_cosmetic.set(false);
                if let Some(plan) = s.hooks.filter.cosmetic_css(&page, st.private) {
                    inject_css(&core, &plan.css);
                    st.generic_cosmetic.set(plan.generic_allowed);
                    *st.cosmetic_exceptions.borrow_mut() = plan.exceptions;
                }
                Ok(())
            })),
            &mut t,
        )?;
        if let Ok(c2) = core.cast::<ICoreWebView2_2>() {
            let (s, st) = (shared.clone(), state.clone());
            c2.add_DOMContentLoaded(
                &DOMContentLoadedEventHandler::create(Box::new(move |sender, _| {
                    if !st.generic_cosmetic.get() {
                        return Ok(());
                    }
                    let Some(core) = sender else { return Ok(()) };
                    let (s, st, core2) = (s.clone(), st.clone(), core.clone());
                    core.ExecuteScript(
                        &HSTRING::from(COLLECT_CLASSES_IDS),
                        &ExecuteScriptCompletedHandler::create(Box::new(move |r, json| {
                            // The page can shape this result (it runs in the page's JS world);
                            // parsing a huge one would stall every window's UI thread.
                            if r.is_err() || json.len() > 256 * 1024 {
                                return Ok(());
                            }
                            #[derive(serde::Deserialize)]
                            struct Found {
                                c: Vec<String>,
                                i: Vec<String>,
                            }
                            if let Ok(found) = serde_json::from_str::<Found>(&json) {
                                let css = s.hooks.filter.generic_css(
                                    &found.c,
                                    &found.i,
                                    &st.cosmetic_exceptions.borrow(),
                                );
                                inject_css(&core2, &css);
                            }
                            Ok(())
                        })),
                    )?;
                    Ok(())
                })),
                &mut t,
            )?;
        }

        // --- favicon (fetched by the engine; handed to the shell as a data URL) ---
        if let Ok(c15) = core.cast::<ICoreWebView2_15>() {
            let s = shared.clone();
            c15.add_FaviconChanged(
                &FaviconChangedEventHandler::create(Box::new(move |sender, _| {
                    let Some(core) = sender else { return Ok(()) };
                    let c15: ICoreWebView2_15 = core.cast()?;
                    let page_url = get_string(|p| core.Source(p));
                    if get_string(|p| c15.FaviconUri(p)).is_empty() {
                        s.emit(EngineEvent::FaviconChanged {
                            tab,
                            page_url,
                            data_url: None,
                        });
                        return Ok(());
                    }
                    let s = s.clone();
                    c15.GetFavicon(
                        COREWEBVIEW2_FAVICON_IMAGE_FORMAT_PNG,
                        &GetFaviconCompletedHandler::create(Box::new(move |r, stream| {
                            let bytes = match (r, stream) {
                                (Ok(()), Some(stream)) => read_stream(&stream, 256 * 1024),
                                _ => Vec::new(),
                            };
                            let data_url = (!bytes.is_empty()).then(|| png_data_url(&bytes));
                            s.emit(EngineEvent::FaviconChanged {
                                tab,
                                page_url,
                                data_url,
                            });
                            Ok(())
                        })),
                    )?;
                    Ok(())
                })),
                &mut t,
            )?;
        }

        // --- popups: become tabs; only user-initiated ones unless the site is allowed ---
        let s = shared.clone();
        core.add_NewWindowRequested(
            &NewWindowRequestedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let url = get_string(|p| args.Uri(p));
                let user_initiated = bool_of(|b| args.IsUserInitiated(b));
                let deferral = args.GetDeferral()?;
                args.SetHandled(true)?;
                // Heuristic: Ctrl/middle-click opens in the background, like other browsers.
                let background = user_initiated
                    && (GetAsyncKeyState(VK_CONTROL.0 as i32) < 0
                        || GetAsyncKeyState(VK_MBUTTON.0 as i32) < 0);
                let request = s.next_request_id();
                s.popups.borrow_mut().insert(request, (args, deferral));
                s.emit(EngineEvent::NewWindowRequested {
                    opener: tab,
                    request,
                    url,
                    user_initiated,
                    background,
                });
                Ok(())
            })),
            &mut t,
        )?;

        let s = shared.clone();
        core.add_WindowCloseRequested(
            &WindowCloseRequestedEventHandler::create(Box::new(move |_, _| {
                s.emit(EngineEvent::CloseRequested { tab });
                Ok(())
            })),
            &mut t,
        )?;

        let s = shared.clone();
        core.add_ContainsFullScreenElementChanged(
            &ContainsFullScreenElementChangedEventHandler::create(Box::new(move |sender, _| {
                if let Some(core) = sender {
                    s.emit(EngineEvent::FullscreenChanged {
                        tab,
                        fullscreen: bool_of(|b| core.ContainsFullScreenElement(b)),
                    });
                }
                Ok(())
            })),
            &mut t,
        )?;

        let s = shared.clone();
        core.add_ProcessFailed(
            &ProcessFailedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
                args.ProcessFailedKind(&mut kind)?;
                if kind == COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED
                    || kind == COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE
                {
                    s.emit(EngineEvent::Crashed { tab });
                }
                Ok(())
            })),
            &mut t,
        )?;

        // --- site permissions: the engine never decides or remembers on its own ---
        let s = shared.clone();
        core.add_PermissionRequested(
            &PermissionRequestedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut native = COREWEBVIEW2_PERMISSION_KIND::default();
                args.PermissionKind(&mut native)?;
                let origin = origin_of(&get_string(|p| args.Uri(p)));
                if let Ok(a3) = args.cast::<ICoreWebView2PermissionRequestedEventArgs3>() {
                    a3.SetSavesInProfile(false)?;
                }
                let (Some(kind), Some(origin)) = (permission_kind(native), origin) else {
                    args.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY)?;
                    return Ok(());
                };
                let deferral = args.GetDeferral()?;
                let request = s.next_request_id();
                s.permissions.borrow_mut().insert(request, (args, deferral));
                s.emit(EngineEvent::PermissionRequested {
                    tab,
                    request,
                    origin,
                    kind,
                });
                Ok(())
            })),
            &mut t,
        )?;

        // --- external protocols (mailto:, zoommtg:, ...): always ask; never launch silently ---
        if let Ok(c18) = core.cast::<ICoreWebView2_18>() {
            let s = shared.clone();
            c18.add_LaunchingExternalUriScheme(
                &LaunchingExternalUriSchemeEventHandler::create(Box::new(move |_, args| {
                    let Some(args) = args else { return Ok(()) };
                    args.SetCancel(true)?;
                    let uri = get_string(|p| args.Uri(p));
                    let scheme = uri.split(':').next().unwrap_or_default();
                    if !is_forbidden_scheme(scheme) {
                        s.emit(EngineEvent::ExternalProtocolRequested { tab, uri });
                    }
                    Ok(())
                })),
                &mut t,
            )?;
        }

        register_context_menu(shared, core, tab)?;
        register_downloads(shared, core, state)?;

        // --- controller-level: keyboard shortcuts, focus traversal, zoom ---
        let s = shared.clone();
        controller.add_AcceleratorKeyPressed(
            &AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut kind = COREWEBVIEW2_KEY_EVENT_KIND::default();
                args.KeyEventKind(&mut kind)?;
                if kind != COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN
                    && kind != COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN
                {
                    return Ok(());
                }
                let mut vk = 0u32;
                args.VirtualKey(&mut vk)?;
                let down = |k: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY| {
                    GetKeyState(k.0 as i32) < 0
                };
                let action = shortcuts::code_from_virtual_key(vk).and_then(|code| {
                    shortcuts::lookup(&code, down(VK_CONTROL), down(VK_SHIFT), down(VK_MENU))
                });
                if let Some(action) = action {
                    args.SetHandled(true)?;
                    s.emit(EngineEvent::Shortcut { tab, action });
                }
                Ok(())
            })),
            &mut t,
        )?;

        let s = shared.clone();
        controller.add_GotFocus(
            &FocusChangedEventHandler::create(Box::new(move |_, _| {
                s.emit(EngineEvent::ViewFocused { tab });
                Ok(())
            })),
            &mut t,
        )?;

        let s = shared.clone();
        controller.add_MoveFocusRequested(
            &MoveFocusRequestedEventHandler::create(Box::new(move |_, args| {
                if let Some(args) = args {
                    args.SetHandled(true)?;
                    s.emit(EngineEvent::FocusLeft { tab });
                }
                Ok(())
            })),
            &mut t,
        )?;

        let s = shared.clone();
        controller.add_ZoomFactorChanged(
            &ZoomFactorChangedEventHandler::create(Box::new(move |sender, _| {
                if let Some(c) = sender {
                    let mut zoom = 1.0;
                    c.ZoomFactor(&mut zoom)?;
                    s.emit(EngineEvent::ZoomChanged { tab, zoom });
                }
                Ok(())
            })),
            &mut t,
        )?;
    }
    Ok(())
}

/// Engine context-menu items that are Edge features or open Edge UI (print preview, DevTools,
/// sharing, web capture...). Links get Veyscape's own "open in new/private tab" items instead
/// of "Open link in new window".
const CONTEXT_MENU_DENY: &[&str] = &[
    "print",
    "inspectElement",
    "moreTools",
    "share",
    "webCapture",
    "webSelect",
    "readAloud",
    "createQrCode",
    "openLinkInNewWindow",
    "openLinkInNewTab",
];

fn register_context_menu(shared: &Rc<Shared>, core: &ICoreWebView2, tab: TabId) -> AppResult<()> {
    let Ok(c11) = core.cast::<ICoreWebView2_11>() else {
        return Ok(());
    };
    let s = shared.clone();
    let mut t = token();
    unsafe {
        c11.add_ContextMenuRequested(
            &ContextMenuRequestedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let items = args.MenuItems()?;
                let mut count = 0u32;
                items.Count(&mut count)?;
                for i in (0..count).rev() {
                    let item = items.GetValueAtIndex(i)?;
                    let name = get_string(|p| item.Name(p));
                    let label = get_string(|p| item.Label(p));
                    log::debug!("context menu item: {name} / {label}");
                    if CONTEXT_MENU_DENY.contains(&name.as_str())
                        || label.contains("Edge")
                        || label == "More tools"
                    {
                        items.RemoveValueAtIndex(i)?;
                    }
                }
                let target = args.ContextMenuTarget()?;
                let link = get_string(|p| target.LinkUri(p));
                if bool_of(|b| target.HasLinkUri(b))
                    && crate::browser::url::web_url(&link).is_some()
                {
                    let env9: ICoreWebView2Environment9 = s.env.cast()?;
                    for (index, (label, private)) in [
                        ("Open link in new tab", false),
                        ("Open link in private tab", true),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        let item = env9.CreateContextMenuItem(
                            &HSTRING::from(label),
                            None,
                            COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND_COMMAND,
                        )?;
                        let (s2, url) = (s.clone(), link.clone());
                        let mut tok = token();
                        item.add_CustomItemSelected(
                            &CustomItemSelectedEventHandler::create(Box::new(move |_, _| {
                                s2.emit(EngineEvent::OpenLink {
                                    opener: tab,
                                    url: url.clone(),
                                    private,
                                });
                                Ok(())
                            })),
                            &mut tok,
                        )?;
                        items.InsertValueAtIndex(index as u32, &item)?;
                    }
                }
                trim_separators(&items)
            })),
            &mut t,
        )?;
    }
    Ok(())
}

/// Editing commands kept in the browser UI's own webviews (text fields and selected text).
const UI_MENU_ALLOW: &[&str] = &[
    "undo",
    "redo",
    "cut",
    "copy",
    "paste",
    "pasteAndMatchStyle",
    "selectAll",
];

/// Browser UI webviews (shell, overlay): no engine page menu (Back, Refresh, Save as, Print,
/// Inspect...). Right-clicking a text field or a selection keeps only the editing commands; the UI
/// shows its own native menus for tabs, groups and bookmarks.
pub fn restrict_ui_context_menu(core: &ICoreWebView2) -> AppResult<()> {
    let c11 = core.cast::<ICoreWebView2_11>()?;
    let mut t = token();
    unsafe {
        c11.add_ContextMenuRequested(
            &ContextMenuRequestedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let target = args.ContextMenuTarget()?;
                let editable = bool_of(|b| target.IsEditable(b));
                let selection = bool_of(|b| target.HasSelection(b));
                if !editable && !selection {
                    // Handled without showing anything: no menu at all.
                    return args.SetHandled(true);
                }
                let items = args.MenuItems()?;
                let mut count = 0u32;
                items.Count(&mut count)?;
                for i in (0..count).rev() {
                    let item = items.GetValueAtIndex(i)?;
                    let name = get_string(|p| item.Name(p));
                    if !is_separator(&item) && !UI_MENU_ALLOW.contains(&name.as_str()) {
                        items.RemoveValueAtIndex(i)?;
                    }
                }
                trim_separators(&items)
            })),
            &mut t,
        )?;
    }
    Ok(())
}

/// Removes separators at the start or end, and doubled ones, left behind after removals.
fn trim_separators(items: &ICoreWebView2ContextMenuItemCollection) -> windows::core::Result<()> {
    let mut count = 0u32;
    unsafe {
        items.Count(&mut count)?;
        let mut i = count;
        let mut previous_was_separator = true; // treats the end like a separator
        while i > 0 {
            i -= 1;
            let sep = is_separator(&items.GetValueAtIndex(i)?);
            if sep && previous_was_separator {
                items.RemoveValueAtIndex(i)?;
            } else {
                previous_was_separator = sep;
            }
        }
        items.Count(&mut count)?;
        if count > 0 && is_separator(&items.GetValueAtIndex(0)?) {
            items.RemoveValueAtIndex(0)?;
        }
    }
    Ok(())
}

fn is_separator(item: &ICoreWebView2ContextMenuItem) -> bool {
    let mut kind = COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND::default();
    unsafe { item.Kind(&mut kind).is_ok() && kind == COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND_SEPARATOR }
}

fn register_downloads(
    shared: &Rc<Shared>,
    core: &ICoreWebView2,
    state: &Rc<ViewState>,
) -> AppResult<()> {
    let tab = state.tab;
    let Ok(c4) = core.cast::<ICoreWebView2_4>() else {
        return Ok(());
    };
    let s = shared.clone();
    let st = state.clone();
    let mut t = token();
    unsafe {
        c4.add_DownloadStarting(
            &DownloadStartingEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                // Suppress the engine's own download UI; the shell shows downloads instead.
                args.SetHandled(true)?;
                st.download_started.set(true);
                let op = args.DownloadOperation()?;
                let id = crate::downloads::next_id();
                let url = get_string(|p| op.Uri(p));
                let path = get_string(|p| args.ResultFilePath(p));
                let mut total = 0i64;
                op.TotalBytesToReceive(&mut total)?;
                let total = (total > 0).then_some(total);

                let last = std::cell::Cell::new(Instant::now() - Duration::from_secs(1));
                let s2 = s.clone();
                let mut tok = token();
                op.add_BytesReceivedChanged(
                    &BytesReceivedChangedEventHandler::create(Box::new(move |sender, _| {
                        // Throttled: progress events can fire hundreds of times per second.
                        if last.get().elapsed() < Duration::from_millis(250) {
                            return Ok(());
                        }
                        last.set(Instant::now());
                        if let Some(op) = sender {
                            let (mut received, mut total) = (0i64, 0i64);
                            op.BytesReceived(&mut received)?;
                            op.TotalBytesToReceive(&mut total)?;
                            s2.emit(EngineEvent::DownloadProgress {
                                id,
                                received,
                                total: (total > 0).then_some(total),
                            });
                        }
                        Ok(())
                    })),
                    &mut tok,
                )?;
                let s3 = s.clone();
                op.add_StateChanged(
                    &StateChangedEventHandler::create(Box::new(move |sender, _| {
                        let Some(op) = sender else { return Ok(()) };
                        let mut native = COREWEBVIEW2_DOWNLOAD_STATE::default();
                        op.State(&mut native)?;
                        let mut received = 0i64;
                        op.BytesReceived(&mut received)?;
                        let path = get_string(|p| op.ResultFilePath(p));
                        let state = if native == COREWEBVIEW2_DOWNLOAD_STATE_COMPLETED {
                            DownloadState::Completed
                        } else if native == COREWEBVIEW2_DOWNLOAD_STATE_INTERRUPTED {
                            let mut reason = COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON::default();
                            op.InterruptReason(&mut reason)?;
                            if reason == COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_USER_CANCELED {
                                DownloadState::Cancelled
                            } else {
                                DownloadState::Interrupted
                            }
                        } else {
                            DownloadState::InProgress
                        };
                        if state == DownloadState::Completed || state == DownloadState::Cancelled {
                            s3.downloads.borrow_mut().remove(&id);
                        }
                        s3.emit(EngineEvent::DownloadFinished {
                            id,
                            state,
                            received,
                            path,
                        });
                        Ok(())
                    })),
                    &mut tok,
                )?;
                s.downloads.borrow_mut().insert(id, op);
                s.emit(EngineEvent::DownloadStarted {
                    tab,
                    id,
                    url,
                    path,
                    total,
                });
                Ok(())
            })),
            &mut t,
        )?;
    }
    Ok(())
}

/// Find-in-page via the WebView2 Find API (runtime 1.0.2849+), with the engine's own find UI suppressed.
pub fn create_find(
    shared: &Rc<Shared>,
    core: &ICoreWebView2,
    tab: TabId,
) -> AppResult<ICoreWebView2Find> {
    let find = unsafe { core.cast::<ICoreWebView2_28>()?.Find()? };
    let report = {
        let s = shared.clone();
        move |f: Option<ICoreWebView2Find>| -> windows::core::Result<()> {
            if let Some(f) = f {
                let (mut matches, mut active) = (0i32, 0i32);
                unsafe {
                    f.MatchCount(&mut matches)?;
                    f.ActiveMatchIndex(&mut active)?;
                }
                s.emit(EngineEvent::FindResult {
                    tab,
                    matches,
                    active,
                });
            }
            Ok(())
        }
    };
    let mut t = token();
    unsafe {
        let r = report.clone();
        find.add_MatchCountChanged(
            &FindMatchCountChangedEventHandler::create(Box::new(move |f, _| r(f))),
            &mut t,
        )?;
        find.add_ActiveMatchIndexChanged(
            &FindActiveMatchIndexChangedEventHandler::create(Box::new(move |f, _| report(f))),
            &mut t,
        )?;
    }
    Ok(find)
}

/// Rewrites `Sec-CH-UA` brand lists so servers are not told the page runs in Microsoft Edge.
fn hide_edge_client_hints(headers: &ICoreWebView2HttpRequestHeaders) {
    for name in ["sec-ch-ua", "sec-ch-ua-full-version-list"] {
        let name = HSTRING::from(name);
        if !bool_of(|b| unsafe { headers.Contains(&name, b) }) {
            continue;
        }
        let value = get_string(|p| unsafe { headers.GetHeader(&name, p) });
        let cleaned = strip_edge_brands(&value);
        if cleaned != value {
            unsafe {
                let _ = headers.SetHeader(&name, &HSTRING::from(cleaned));
            }
        }
    }
}

/// Errors worth an error screen. Cancellations and aborts (Stop, a newer navigation, a
/// download, a page rejecting itself) leave the previous state alone.
fn load_failure_reason(status: COREWEBVIEW2_WEB_ERROR_STATUS) -> Option<&'static str> {
    Some(match status {
        COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_COMMON_NAME_IS_INCORRECT
        | COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_EXPIRED
        | COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_IS_INVALID
        | COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_REVOKED
        | COREWEBVIEW2_WEB_ERROR_STATUS_CLIENT_CERTIFICATE_CONTAINS_ERRORS => "certificate",
        COREWEBVIEW2_WEB_ERROR_STATUS_HOST_NAME_NOT_RESOLVED => "notFound",
        COREWEBVIEW2_WEB_ERROR_STATUS_TIMEOUT => "timeout",
        COREWEBVIEW2_WEB_ERROR_STATUS_SERVER_UNREACHABLE
        | COREWEBVIEW2_WEB_ERROR_STATUS_CANNOT_CONNECT => "unreachable",
        COREWEBVIEW2_WEB_ERROR_STATUS_DISCONNECTED => "offline",
        COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_RESET
        | COREWEBVIEW2_WEB_ERROR_STATUS_ERROR_HTTP_INVALID_SERVER_RESPONSE
        | COREWEBVIEW2_WEB_ERROR_STATUS_REDIRECT_FAILED
        | COREWEBVIEW2_WEB_ERROR_STATUS_UNEXPECTED_ERROR
        | COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN => "failed",
        _ => return None,
    })
}

fn resource_kind(ctx: COREWEBVIEW2_WEB_RESOURCE_CONTEXT) -> ResourceKind {
    match ctx {
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT => ResourceKind::Subdocument,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_STYLESHEET => ResourceKind::Stylesheet,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_IMAGE => ResourceKind::Image,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_MEDIA | COREWEBVIEW2_WEB_RESOURCE_CONTEXT_TEXT_TRACK => {
            ResourceKind::Media
        }
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FONT => ResourceKind::Font,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_SCRIPT => ResourceKind::Script,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_XML_HTTP_REQUEST
        | COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FETCH
        | COREWEBVIEW2_WEB_RESOURCE_CONTEXT_EVENT_SOURCE => ResourceKind::Xhr,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_PING
        | COREWEBVIEW2_WEB_RESOURCE_CONTEXT_CSP_VIOLATION_REPORT => ResourceKind::Ping,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_WEBSOCKET => ResourceKind::Websocket,
        _ => ResourceKind::Other,
    }
}

fn permission_kind(k: COREWEBVIEW2_PERMISSION_KIND) -> Option<PermissionKind> {
    Some(match k {
        COREWEBVIEW2_PERMISSION_KIND_CAMERA => PermissionKind::Camera,
        COREWEBVIEW2_PERMISSION_KIND_MICROPHONE => PermissionKind::Microphone,
        COREWEBVIEW2_PERMISSION_KIND_GEOLOCATION => PermissionKind::Location,
        COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS => PermissionKind::Notifications,
        COREWEBVIEW2_PERMISSION_KIND_MULTIPLE_AUTOMATIC_DOWNLOADS => {
            PermissionKind::AutomaticDownloads
        }
        // Clipboard read, sensors, MIDI SysEx, local fonts, window management, file system: denied.
        _ => return None,
    })
}

/// Collects class names and ids present on the page (bounded per class: one element may carry
/// millions) for generic cosmetic rules.
const COLLECT_CLASSES_IDS: &str = "(()=>{const c=new Set(),i=new Set();\
o:for(const e of document.querySelectorAll('[class],[id]')){\
for(const x of e.classList){if(c.size+i.size>6000)break o;c.add(x);}\
if(e.id)i.add(e.id);}return{c:[...c],i:[...i]};})()";

fn inject_css(core: &ICoreWebView2, css: &str) {
    if css.is_empty() {
        return;
    }
    // The CSS travels as a JSON string literal, so it cannot break out of the script.
    let literal = serde_json::to_string(css).unwrap_or_else(|_| "\"\"".into());
    let script = format!(
        "(()=>{{const s=document.createElement('style');s.textContent={literal};(document.head||document.documentElement).appendChild(s);}})()"
    );
    unsafe {
        let _ = core.ExecuteScript(
            &HSTRING::from(script),
            &ExecuteScriptCompletedHandler::create(Box::new(|_, _| Ok(()))),
        );
    }
}
