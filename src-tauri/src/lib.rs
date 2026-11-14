//! Veyscape: a privacy-first desktop browser. Module map: see docs/architecture.md.

pub mod adblock;
pub mod bookmarks;
pub mod browser;
pub mod commands;
pub mod config;
pub mod downloads;
pub mod errors;
pub mod passwords;
pub mod permissions;
pub mod platform;
pub mod privacy;
pub mod shell;
pub mod state;
pub mod storage;

use std::sync::{Arc, RwLock};
use tauri::Manager;

/// Minimal stderr logger. Level via `VEYSCAPE_LOG` (error|warn|info|debug); default warn.
/// Never log URLs with query strings, cookies or form data.
struct Logger(log::LevelFilter);

impl log::Log for Logger {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= self.0
    }
    fn log(&self, r: &log::Record) {
        if self.enabled(r.metadata()) {
            eprintln!("[{} {}] {}", r.level(), r.target(), r.args());
        }
    }
    fn flush(&self) {}
}

fn init_logging() {
    let level = match std::env::var("VEYSCAPE_LOG").as_deref() {
        Ok("error") => log::LevelFilter::Error,
        Ok("info") => log::LevelFilter::Info,
        Ok("debug") => log::LevelFilter::Debug,
        _ => log::LevelFilter::Warn,
    };
    let logger: &'static Logger = Box::leak(Box::new(Logger(level)));
    if log::set_logger(logger).is_ok() {
        log::set_max_level(level);
    }
}

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.handle().clone();
    let paths = config::Paths::resolve(&handle)?;
    let db = Arc::new(storage::Db::open(&paths.database)?);
    db.downloads_mark_stale_interrupted()?;
    downloads::init_ids(&db);
    let settings = db.settings_load()?;
    let adblock = Arc::new(adblock::AdBlockEngine::new(
        db.clone(),
        paths.filter_cache.clone(),
        settings.adblock_enabled,
        settings.cosmetic_filtering,
    )?);

    // Load the compiled engine from cache synchronously when present (milliseconds), so the
    // first page is filtered; list downloads and rebuilds happen in the background.
    adblock.initialize();
    // Lists go stale after a few days; check now and every few hours while the browser stays
    // open (a check with nothing stale downloads nothing).
    let a = adblock.clone();
    std::thread::spawn(move || loop {
        if let Err(e) = a.update_lists(false) {
            log::warn!("filter list update: {e}");
        }
        std::thread::sleep(std::time::Duration::from_secs(6 * 60 * 60));
    });
    let notify = handle.clone();
    adblock.set_change_hook(Box::new(move || {
        use tauri::Emitter;
        let _ = notify.emit("protection:changed", ());
    }));

    let services = Arc::new(state::Services {
        db,
        adblock,
        settings_cell: Arc::new(RwLock::new(settings)),
    });
    app.manage(services.clone());
    browser::install(browser::Browser::new(handle.clone(), services.clone()));
    for seed in browser::startup_seeds(&services) {
        shell::create_window(&handle, seed)?;
    }
    Ok(())
}

pub fn run() {
    init_logging();
    let handler: fn(tauri::ipc::Invoke) -> bool = tauri::generate_handler![
        commands::browser::browser_snapshot,
        commands::browser::window_new,
        commands::browser::tab_create,
        commands::browser::tab_close,
        commands::browser::tab_activate,
        commands::browser::tab_move,
        commands::browser::tab_tear_off,
        commands::browser::tab_drag_start,
        commands::browser::tab_drop,
        commands::browser::tab_reopen_closed,
        commands::browser::tab_context_menu,
        commands::browser::tab_group_menu,
        commands::browser::tab_group_update,
        commands::browser::split_set,
        commands::browser::split_close,
        commands::browser::menu_open_main,
        commands::browser::nav_go,
        commands::browser::nav_back,
        commands::browser::nav_forward,
        commands::browser::nav_reload,
        commands::browser::nav_stop,
        commands::browser::zoom_change,
        commands::browser::find_in_page,
        commands::browser::find_stop,
        commands::browser::print_page,
        commands::browser::focus_content,
        commands::browser::https_continue,
        commands::browser::layout_update,
        commands::browser::overlay_show,
        commands::browser::overlay_hide,
        commands::browser::permission_respond,
        commands::browser::external_protocol_respond,
        commands::passwords::passwords_list,
        commands::passwords::passwords_for_tab,
        commands::passwords::password_fill,
        commands::passwords::password_reveal,
        commands::passwords::password_update,
        commands::passwords::password_delete,
        commands::passwords::passwords_never_list,
        commands::passwords::passwords_never_remove,
        commands::library::suggest,
        commands::library::top_sites,
        commands::library::bookmarks_list,
        commands::library::bookmark_toggle,
        commands::library::bookmark_update,
        commands::library::bookmark_remove,
        commands::library::bookmark_add,
        commands::library::bookmark_bar_menu,
        commands::library::bookmark_create_folder,
        commands::library::bookmark_move,
        commands::library::bookmark_import_sources,
        commands::library::bookmark_import_browser,
        commands::library::bookmark_import_file,
        commands::library::bookmark_folder_menu,
        commands::library::bookmark_item_menu,
        commands::library::history_list,
        commands::library::history_delete,
        commands::library::downloads_list,
        commands::library::download_cancel,
        commands::library::download_resume,
        commands::library::download_open,
        commands::library::download_show,
        commands::library::download_remove,
        commands::library::downloads_clear,
        commands::settings::settings_get,
        commands::settings::settings_update,
        commands::settings::protection_status,
        commands::settings::adblock_set_enabled,
        commands::settings::adblock_set_site_enabled,
        commands::settings::site_exceptions_list,
        commands::settings::filter_lists,
        commands::settings::filter_list_add,
        commands::settings::filter_list_remove,
        commands::settings::filter_list_set_enabled,
        commands::settings::filter_lists_update,
        commands::settings::custom_rules_get,
        commands::settings::custom_rules_set,
        commands::settings::clear_browsing_data,
        commands::settings::permissions_list,
        commands::settings::permission_set,
        commands::settings::app_info,
    ];

    let app = tauri::Builder::default()
        // Launching Veyscape again opens a window in the running instance, so all windows share
        // one profile, history and session instead of two processes fighting over them.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = shell::create_window(&app, browser::WindowSeed::New) {
                    log::error!("opening a window for a second launch failed: {e}");
                }
            });
        }))
        .plugin(tauri_plugin_opener::init())
        .on_menu_event(shell::on_menu_event)
        .invoke_handler(move |invoke| {
            // Defense in depth on top of the capability ACL: only the trusted UI may call in.
            if !commands::is_trusted_caller(invoke.message.webview_ref().label()) {
                invoke.resolver.reject("caller is not allowed");
                return true;
            }
            handler(invoke)
        })
        .setup(setup)
        .build(tauri::generate_context!())
        .expect("failed to build Veyscape");

    app.run(|app, event| {
        if let tauri::RunEvent::ExitRequested { api, .. } = event {
            let _ = browser::with_browser(|b| b.prepare_exit());
            let clear = app
                .state::<Arc<state::Services>>()
                .settings()
                .clear_cookies_on_exit;
            static CLEARING: std::sync::atomic::AtomicBool =
                std::sync::atomic::AtomicBool::new(false);
            if clear && !CLEARING.swap(true, std::sync::atomic::Ordering::SeqCst) {
                // Hold the exit until the engine has wiped cookies and site data.
                api.prevent_exit();
                let app2 = app.clone();
                let kinds = platform::ClearDataKinds {
                    cookies: true,
                    ..Default::default()
                };
                let started = browser::with_browser(|b| {
                    b.clear_data(kinds, Box::new(move |_| app2.exit(0)));
                });
                if started.is_err() {
                    app.exit(0);
                }
            }
        }
    });
}
