//! Saved logins: Settings → Passwords, the key button popup and filling. Passwords reach the UI
//! only through `password_reveal`, after Windows Hello confirms the user.

use crate::browser::with_caller;
use crate::errors::{AppError, AppResult};
use crate::passwords;
use crate::platform::TabId;
use crate::state::Services;
use crate::storage::Login;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, State, Webview};

type Svc<'a> = State<'a, Arc<Services>>;

/// Asks Windows Hello before a saved password is shown or changed. One confirmation covers the
/// next minute, so revealing and then editing asks once.
async fn confirm_user(webview: &Webview) -> AppResult<()> {
    static CONFIRMED: Mutex<Option<Instant>> = Mutex::new(None);
    let recent = || {
        CONFIRMED
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_some_and(|t| t.elapsed() < Duration::from_secs(60))
    };
    if recent() {
        return Ok(());
    }
    let window = webview.window();
    let confirmed = tauri::async_runtime::spawn_blocking(move || {
        passwords::verify_user(
            &window,
            "Veyscape wants to show or change a saved password.",
        )
    })
    .await
    .map_err(|e| AppError::Engine(e.to_string()))??;
    if !confirmed {
        return Err(AppError::Denied(
            "Windows Hello did not confirm it's you.".into(),
        ));
    }
    *CONFIRMED.lock().unwrap_or_else(|p| p.into_inner()) = Some(Instant::now());
    Ok(())
}

fn changed(app: &AppHandle) {
    let _ = app.emit("passwords:changed", ());
}

#[tauri::command]
pub fn passwords_list(svc: Svc<'_>) -> AppResult<Vec<Login>> {
    svc.db.logins_list()
}

/// Logins saved for the site open in `tab_id` (the key button popup).
#[tauri::command]
pub fn passwords_for_tab(webview: Webview, tab_id: TabId) -> AppResult<Vec<Login>> {
    with_caller(&webview, |w| w.logins_for_tab(tab_id))?
}

/// Resolves once the page reports whether the form was filled, so the popup can show a failure.
#[tauri::command]
pub async fn password_fill(
    app: AppHandle,
    webview: Webview,
    tab_id: TabId,
    id: i64,
) -> AppResult<()> {
    super::answered_on_ui_thread(
        &app,
        Duration::from_secs(10),
        "the page did not answer the fill request",
        move |done| with_caller(&webview, |w| w.fill_login(tab_id, id, done)).and_then(|r| r),
    )
    .await
}

#[tauri::command]
pub async fn password_reveal(webview: Webview, svc: Svc<'_>, id: i64) -> AppResult<String> {
    confirm_user(&webview).await?;
    let (_, _, blob) = svc
        .db
        .login_secret(id)?
        .ok_or_else(|| AppError::NotFound("login".into()))?;
    passwords::unprotect(&blob)
}

#[tauri::command]
pub async fn password_update(
    app: AppHandle,
    webview: Webview,
    svc: Svc<'_>,
    id: i64,
    username: String,
    password: String,
) -> AppResult<()> {
    let username = username.trim();
    if !passwords::valid_login(username, &password) {
        return Err(AppError::InvalidInput(
            "the password must not be empty, and both fields must be shorter".into(),
        ));
    }
    confirm_user(&webview).await?;
    svc.db
        .login_update(id, username, &passwords::protect(&password)?)?;
    changed(&app);
    Ok(())
}

/// Async: the WAL checkpoint after the deletion must not run on the UI thread.
#[tauri::command]
pub async fn password_delete(app: AppHandle, svc: Svc<'_>, id: i64) -> AppResult<()> {
    svc.db.login_delete(id)?;
    changed(&app);
    Ok(())
}

/// Sites the user chose "Never" for.
#[tauri::command]
pub fn passwords_never_list(svc: Svc<'_>) -> AppResult<Vec<String>> {
    svc.db.login_never_list()
}

#[tauri::command]
pub fn passwords_never_remove(app: AppHandle, svc: Svc<'_>, origin: String) -> AppResult<()> {
    svc.db.login_never_set(&origin, false)?;
    changed(&app);
    Ok(())
}
