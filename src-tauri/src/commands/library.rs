//! Bookmarks, history, downloads, address-bar suggestions.

use crate::browser::{url, with_browser, with_caller};
use crate::errors::{AppError, AppResult};
use crate::platform::TabId;
use crate::state::Services;
use crate::storage::{Bookmark, HistoryEntry, ImportSummary, Suggestion};
use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State, Webview};
use tauri_plugin_opener::OpenerExt;

type Svc<'a> = State<'a, Arc<Services>>;

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Suggestions {
    items: Vec<Suggestion>,
    /// The typed text completed to a site the user visits (see `url::inline_completion`).
    completion: Option<String>,
}

/// Local-only suggestions from history and bookmarks. Typed text never leaves the machine.
/// Async so the LIKE scans (milliseconds on a big history, every keystroke) stay off the UI
/// thread, which every page request waits on.
#[tauri::command]
pub async fn suggest(svc: Svc<'_>, query: String) -> AppResult<Suggestions> {
    let q = query.trim();
    if q.is_empty() || q.len() > 512 {
        return Ok(Suggestions::default());
    }
    let completion = match url::can_complete(&query) {
        true => url::inline_completion(&query, &svc.db.completion_candidates(&query)?),
        false => None,
    };
    Ok(Suggestions {
        items: svc.db.suggest(q, 6)?,
        completion,
    })
}

/// Async (like `suggest`) so the history scan stays off the UI thread.
#[tauri::command]
pub async fn top_sites(svc: Svc<'_>) -> AppResult<Vec<HistoryEntry>> {
    svc.db.history_top_sites(8)
}

#[tauri::command]
pub fn bookmarks_list(svc: Svc<'_>) -> AppResult<Vec<Bookmark>> {
    svc.db.bookmarks_list()
}

/// Tells every window's UI (bookmark bar, pages) and the core (star icons) that bookmarks changed.
fn bookmarks_changed(app: &AppHandle) -> AppResult<()> {
    let _ = app.emit("bookmarks:changed", ());
    // Async commands run off the UI thread; the browser core must be touched on it.
    let _ = app.run_on_main_thread(|| {
        let _ = with_browser(|b| b.refresh_bookmarks());
    });
    Ok(())
}

fn clean_title(title: &str) -> AppResult<&str> {
    let title = title.trim();
    if title.is_empty() || title.len() > 1024 {
        return Err(AppError::InvalidInput(
            "title must be 1–1024 characters".into(),
        ));
    }
    Ok(title)
}

/// Toggles the bookmark for a tab's current page; returns the new saved state.
#[tauri::command]
pub fn bookmark_toggle(app: AppHandle, webview: Webview, id: TabId) -> AppResult<bool> {
    let saved = with_caller(&webview, |b| b.toggle_bookmark(id))??;
    bookmarks_changed(&app)?;
    Ok(saved)
}

#[tauri::command]
pub fn bookmark_update(
    app: AppHandle,
    svc: Svc<'_>,
    id: i64,
    title: String,
    icon: Option<String>,
    color: Option<String>,
    url: Option<String>,
) -> AppResult<()> {
    svc.db.bookmark_update(
        id,
        clean_title(&title)?,
        icon.as_deref(),
        color.as_deref(),
        url.as_deref(),
    )?;
    bookmarks_changed(&app)
}

/// Custom bookmark with a user-entered address.
#[tauri::command]
pub fn bookmark_add(
    app: AppHandle,
    svc: Svc<'_>,
    parent: Option<i64>,
    title: String,
    url: String,
) -> AppResult<i64> {
    if title.len() > 1024 || url.len() > 8 * 1024 {
        return Err(AppError::InvalidInput(
            "title or address is too long".into(),
        ));
    }
    let id = svc.db.bookmark_add(parent, &title, &url)?;
    bookmarks_changed(&app)?;
    Ok(id)
}

/// Right-click on the bookmarks bar background.
#[tauri::command]
pub async fn bookmark_bar_menu(
    webview: Webview,
    can_bookmark_page: bool,
    x: f64,
    y: f64,
) -> AppResult<()> {
    crate::shell::popup_bookmark_bar_menu(&webview.window(), can_bookmark_page, x, y)
}

#[tauri::command]
pub fn bookmark_create_folder(
    app: AppHandle,
    svc: Svc<'_>,
    parent: Option<i64>,
    title: String,
    icon: Option<String>,
    color: Option<String>,
) -> AppResult<i64> {
    let id = svc.db.bookmark_create_folder(
        parent,
        clean_title(&title)?,
        icon.as_deref(),
        color.as_deref(),
    )?;
    bookmarks_changed(&app)?;
    Ok(id)
}

#[tauri::command]
pub fn bookmark_move(app: AppHandle, svc: Svc<'_>, id: i64, parent: Option<i64>) -> AppResult<()> {
    svc.db.bookmark_move(id, parent)?;
    bookmarks_changed(&app)
}

#[tauri::command]
pub fn bookmark_remove(app: AppHandle, svc: Svc<'_>, id: i64) -> AppResult<()> {
    svc.db.bookmark_remove(id)?;
    bookmarks_changed(&app)
}

#[tauri::command]
pub fn bookmark_import_sources() -> Vec<crate::bookmarks::ImportSource> {
    crate::bookmarks::sources()
}

/// Imports another browser's bookmarks into a new folder on the bar.
#[tauri::command]
pub async fn bookmark_import_browser(
    app: AppHandle,
    svc: Svc<'_>,
    id: String,
) -> AppResult<ImportSummary> {
    let db = svc.db.clone();
    let summary = tauri::async_runtime::spawn_blocking(move || -> AppResult<ImportSummary> {
        let (name, nodes) = crate::bookmarks::read_browser(&id)?;
        db.bookmarks_import(&format!("Imported from {name}"), nodes)
    })
    .await
    .map_err(|e| AppError::Engine(e.to_string()))??;
    bookmarks_changed(&app)?;
    Ok(summary)
}

/// Imports an exported bookmarks file (HTML or JSON) the user picked in the shell.
#[tauri::command]
pub async fn bookmark_import_file(
    app: AppHandle,
    svc: Svc<'_>,
    name: String,
    content: String,
) -> AppResult<ImportSummary> {
    let db = svc.db.clone();
    let label: String = name.chars().take(60).collect();
    // Parsing and icon decoding of a large file take a while; keep them off the async workers.
    let summary = tauri::async_runtime::spawn_blocking(move || -> AppResult<ImportSummary> {
        let nodes = crate::bookmarks::parse_file(&content)?;
        db.bookmarks_import(&format!("Imported from {label}"), nodes)
    })
    .await
    .map_err(|e| AppError::Engine(e.to_string()))??;
    bookmarks_changed(&app)?;
    Ok(summary)
}

/// Native dropdown with a bar folder's contents (draws above web pages, keyboard accessible).
#[tauri::command]
pub async fn bookmark_folder_menu(
    webview: Webview,
    svc: Svc<'_>,
    id: i64,
    x: f64,
    y: f64,
) -> AppResult<()> {
    let all = svc.db.bookmarks_list()?;
    crate::shell::popup_bookmark_folder(&webview.window(), &all, id, x, y)
}

/// Context menu for a bookmark bar item.
#[tauri::command]
pub async fn bookmark_item_menu(
    webview: Webview,
    svc: Svc<'_>,
    id: i64,
    x: f64,
    y: f64,
) -> AppResult<()> {
    let item = svc.db.bookmark_get(id)?;
    crate::shell::popup_bookmark_item(&webview.window(), &item, x, y)
}

/// Async (like `suggest`) so the LIKE scan stays off the UI thread.
#[tauri::command]
pub async fn history_list(
    svc: Svc<'_>,
    query: String,
    offset: u32,
    limit: u32,
) -> AppResult<Vec<HistoryEntry>> {
    svc.db
        .history_list(query.trim(), limit.clamp(1, 1000), offset)
}

/// History is shared by all windows; their History pages reload on this event.
pub fn history_changed(app: &AppHandle) {
    let _ = app.emit("history:changed", ());
}

/// Async: the WAL checkpoint after the deletion must not run on the UI thread.
#[tauri::command]
pub async fn history_delete(app: AppHandle, svc: Svc<'_>, id: i64) -> AppResult<()> {
    svc.db.history_delete(id)?;
    history_changed(&app);
    Ok(())
}

#[tauri::command]
pub fn downloads_list() -> AppResult<Vec<crate::downloads::DownloadItem>> {
    with_browser(|b| b.downloads().to_vec())
}

#[tauri::command]
pub fn download_cancel(id: i64) -> AppResult<()> {
    with_browser(|b| b.cancel_download(id))?
}

#[tauri::command]
pub fn download_resume(id: i64) -> AppResult<()> {
    with_browser(|b| b.resume_download(id))?
}

/// Opens a finished download with its default app. The shell confirms risky types first;
/// Windows still applies Mark-of-the-Web / SmartScreen checks when the file is launched.
#[tauri::command]
pub fn download_open(app: AppHandle, id: i64) -> AppResult<()> {
    let path = with_browser(|b| b.download_path(id))??;
    if !std::path::Path::new(&path).is_file() {
        return Err(AppError::NotFound("the file was moved or deleted".into()));
    }
    app.opener()
        .open_path(&path, None::<&str>)
        .map_err(|e| AppError::Engine(e.to_string()))
}

#[tauri::command]
pub fn download_show(app: AppHandle, id: i64) -> AppResult<()> {
    let path = with_browser(|b| b.download_path(id))??;
    app.opener()
        .reveal_item_in_dir(&path)
        .map_err(|e| AppError::Engine(e.to_string()))
}

#[tauri::command]
pub fn download_remove(id: i64) -> AppResult<()> {
    with_browser(|b| b.remove_download(id))?
}

#[tauri::command]
pub fn downloads_clear() -> AppResult<()> {
    with_browser(|b| b.clear_downloads(0))?
}
