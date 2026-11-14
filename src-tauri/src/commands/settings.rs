//! Settings, protection (ad blocking), privacy controls, site permissions, About info.

use crate::adblock::{EngineStatus, FilterList};
use crate::browser::{url::host_of, with_browser};
use crate::config::{Settings, TrackingPrevention};
use crate::errors::{AppError, AppResult};
use crate::permissions::{self, Decision, PermissionKind};
use crate::platform::TabId;
use crate::privacy::{self, ClearDataRequest};
use crate::state::Services;
use crate::storage::SitePermission;
use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

type Svc<'a> = State<'a, Arc<Services>>;

#[tauri::command]
pub fn settings_get(svc: Svc<'_>) -> Settings {
    svc.settings()
}

#[tauri::command]
pub fn settings_update(app: AppHandle, svc: Svc<'_>, settings: Settings) -> AppResult<Settings> {
    svc.replace_settings(settings.clone())?;
    with_browser(|b| b.apply_settings(&settings))?;
    let _ = app.emit("settings:changed", &settings);
    Ok(settings)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtectionStatus {
    adblock_enabled: bool,
    /// `None` for pages where per-site exceptions do not apply (internal pages).
    host: Option<String>,
    site_enabled: bool,
    blocked_on_page: u32,
    tracking_prevention: TrackingPrevention,
    /// The tab is private: site choices last only for the private session.
    private: bool,
    engine: EngineStatus,
}

#[tauri::command]
pub fn protection_status(svc: Svc<'_>, id: Option<TabId>) -> AppResult<ProtectionStatus> {
    let (host, blocked, private) = match id {
        Some(id) => with_browser(|b| {
            b.tab(id)
                .map(|t| (host_of(&t.url), t.blocked, t.private))
                .unwrap_or((None, 0, false))
        })?,
        None => (None, 0, false),
    };
    let s = svc.settings();
    Ok(ProtectionStatus {
        adblock_enabled: s.adblock_enabled,
        site_enabled: host
            .as_deref()
            .is_none_or(|h| svc.adblock.site_enabled(h, private)),
        host,
        blocked_on_page: blocked,
        // Private tabs always run with strict tracking prevention (`ProfilePrefs::for_view`).
        tracking_prevention: if private {
            TrackingPrevention::Strict
        } else {
            s.tracking_prevention
        },
        private,
        engine: svc.adblock.status(),
    })
}

#[tauri::command]
pub fn adblock_set_enabled(app: AppHandle, svc: Svc<'_>, enabled: bool) -> AppResult<()> {
    let next = Settings {
        adblock_enabled: enabled,
        ..svc.settings()
    };
    svc.replace_settings(next.clone())?;
    // Every window keeps a copy of the settings and sends it back whole on its next change.
    let _ = app.emit("settings:changed", &next);
    Ok(())
}

/// `tab`: the tab the choice was made in. From a private tab it is kept in memory for private
/// tabs only, so a site visited privately never lands in the saved exceptions list.
#[tauri::command]
pub fn adblock_set_site_enabled(
    svc: Svc<'_>,
    host: String,
    enabled: bool,
    tab: Option<TabId>,
) -> AppResult<()> {
    let private = match tab {
        Some(id) => with_browser(|b| b.tab(id).is_none_or(|t| t.private))?,
        None => false,
    };
    if private {
        svc.adblock.set_private_site_exception(&host, !enabled)
    } else {
        svc.adblock.set_site_exception(&host, !enabled)
    }
}

#[tauri::command]
pub fn site_exceptions_list(svc: Svc<'_>) -> Vec<String> {
    svc.adblock.site_exceptions()
}

#[tauri::command]
pub fn filter_lists(svc: Svc<'_>) -> Vec<FilterList> {
    svc.adblock.lists()
}

/// Runs a blocking ad-block job off the UI thread.
async fn background<T: Send + 'static>(
    svc: &Arc<Services>,
    job: impl FnOnce(&crate::adblock::AdBlockEngine) -> AppResult<T> + Send + 'static,
) -> AppResult<T> {
    let adblock = svc.adblock.clone();
    tauri::async_runtime::spawn_blocking(move || job(&adblock))
        .await
        .map_err(|e| AppError::Engine(e.to_string()))?
}

#[tauri::command]
pub async fn filter_list_add(svc: Svc<'_>, url: String, title: String) -> AppResult<FilterList> {
    let list = svc.adblock.add_list(url.trim(), &title)?;
    background(&svc, |a| a.update_lists(false)).await?;
    Ok(list)
}

#[tauri::command]
pub async fn filter_list_remove(svc: Svc<'_>, id: String) -> AppResult<()> {
    svc.adblock.remove_list(&id)?;
    background(&svc, |a| a.rebuild()).await
}

#[tauri::command]
pub async fn filter_list_set_enabled(svc: Svc<'_>, id: String, enabled: bool) -> AppResult<()> {
    svc.adblock.set_list_enabled(&id, enabled)?;
    // Enabling a never-downloaded list fetches it; otherwise just recompile.
    background(&svc, |a| a.update_lists(false)).await
}

#[tauri::command]
pub async fn filter_lists_update(svc: Svc<'_>) -> AppResult<()> {
    background(&svc, |a| a.update_lists(true)).await
}

#[tauri::command]
pub fn custom_rules_get(svc: Svc<'_>) -> AppResult<String> {
    svc.adblock.custom_rules()
}

#[tauri::command]
pub async fn custom_rules_set(svc: Svc<'_>, rules: String) -> AppResult<()> {
    svc.adblock.set_custom_rules(&rules)?;
    background(&svc, |a| a.rebuild()).await
}

#[tauri::command]
pub async fn clear_browsing_data(
    app: AppHandle,
    svc: Svc<'_>,
    request: ClearDataRequest,
) -> AppResult<()> {
    privacy::clear_local(&svc.db, &request)?;
    if request.history {
        super::library::history_changed(&app);
    }
    let kinds = request.engine_kinds();
    let clear_downloads = request.downloads;
    super::answered_on_ui_thread(
        &app,
        std::time::Duration::from_secs(60),
        "clearing site data timed out",
        move |done| {
            with_browser(|b| {
                // The list on screen too, not only the saved rows.
                if clear_downloads {
                    let _ = b.clear_downloads(kinds.since_ms);
                }
                b.clear_data(kinds, done)
            })
        },
    )
    .await
}

#[tauri::command]
pub fn permissions_list(svc: Svc<'_>) -> AppResult<Vec<SitePermission>> {
    svc.db.permissions_list()
}

/// `state`: "allow", "deny", or null to forget the decision.
#[tauri::command]
pub fn permission_set(
    svc: Svc<'_>,
    origin: String,
    kind: String,
    state: Option<String>,
) -> AppResult<()> {
    let kind = PermissionKind::parse(&kind)?;
    let decision = match state.as_deref() {
        Some("allow") => Some(Decision::Allow),
        Some("deny") => Some(Decision::Deny),
        None => None,
        Some(_) => {
            return Err(AppError::InvalidInput(
                "state must be allow, deny or null".into(),
            ))
        }
    };
    permissions::remember(&svc.db, &origin, kind, decision)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    version: &'static str,
    tauri_version: &'static str,
    engine: String,
    platform: &'static str,
}

#[tauri::command]
pub fn app_info() -> AppResult<AppInfo> {
    Ok(AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        tauri_version: tauri::VERSION,
        engine: with_browser(|b| b.engine_version())?,
        platform: std::env::consts::OS,
    })
}
