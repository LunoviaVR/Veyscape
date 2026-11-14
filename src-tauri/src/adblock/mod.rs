//! Ad-blocking engine built on Brave's `adblock-rust` (MPL-2.0).
//!
//! Independent of any WebView: platform adapters call [`AdBlockEngine::should_block`] from their
//! request-interception hook and [`AdBlockEngine::cosmetic_css`] when a page starts loading.
//! Engines are compiled off the UI thread and cached as a serialized blob, so normal startup
//! only deserializes (no list parsing).

mod lists;

pub use lists::FilterList;

use crate::browser::url::host_of;
use crate::errors::{AppError, AppResult};
use crate::storage::{now_millis, Db};
use adblock::lists::{FilterSet, ParseOptions};
use adblock::request::Request;
use adblock::Engine;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock, TryLockError};

/// Request categories the platform layer can report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceKind {
    Subdocument,
    Stylesheet,
    Image,
    Media,
    Font,
    Script,
    Xhr,
    Ping,
    Websocket,
    Other,
}

impl ResourceKind {
    fn as_adblock_str(self) -> &'static str {
        match self {
            ResourceKind::Subdocument => "sub_frame",
            ResourceKind::Stylesheet => "stylesheet",
            ResourceKind::Image => "image",
            ResourceKind::Media => "media",
            ResourceKind::Font => "font",
            ResourceKind::Script => "script",
            ResourceKind::Xhr => "xhr",
            ResourceKind::Ping => "ping",
            ResourceKind::Websocket => "websocket",
            ResourceKind::Other => "other",
        }
    }
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    /// True once an engine with rules is active (cached or freshly built).
    pub ready: bool,
    pub updating: bool,
    pub rule_lists_active: usize,
    pub last_error: Option<String>,
    pub total_blocked: u64,
}

/// Cosmetic resources for one page load.
pub struct CosmeticPlan {
    pub css: String,
    pub generic_allowed: bool,
    pub exceptions: HashSet<String>,
}

type ChangeHook = Box<dyn Fn() + Send + Sync>;

pub struct AdBlockEngine {
    engine: RwLock<Option<Arc<Engine>>>,
    enabled: AtomicBool,
    cosmetic: AtomicBool,
    exceptions: RwLock<HashSet<String>>,
    /// Site on/off choices made in private tabs (host → excepted). Memory only, apply to private
    /// tabs only, and are dropped when the last private tab closes.
    private_overrides: RwLock<HashMap<String, bool>>,
    total_blocked: AtomicU64,
    status: Mutex<EngineStatus>,
    /// Serializes rebuilds so two updates never race on the cache files.
    rebuild_lock: Mutex<()>,
    /// Held while a list update runs; a second update returns at once.
    update_lock: Mutex<()>,
    /// An update was asked for while one ran (a list toggled or added): run again afterwards.
    rerun: AtomicBool,
    /// Held around every load → modify → save of the list catalogue, so edits are never lost.
    catalogue_lock: Mutex<()>,
    /// Fingerprint of the installed engine's inputs; lets a no-op update skip the rebuild.
    built_from: Mutex<String>,
    cache_dir: PathBuf,
    db: Arc<Db>,
    on_change: RwLock<Option<ChangeHook>>,
}

const KV_EXCEPTIONS: &str = "adblock.exceptions";
const KV_CUSTOM_RULES: &str = "adblock.customRules";
const ENGINE_FILE: &str = "engine.dat";
const ENGINE_META_FILE: &str = "engine.meta";

fn clean_host(host: &str) -> AppResult<String> {
    let host = host.trim().to_ascii_lowercase();
    if host.is_empty() || host.contains(['/', ' ', ':']) {
        return Err(AppError::InvalidInput("invalid host".into()));
    }
    Ok(host)
}

impl AdBlockEngine {
    /// Creates the service with protection state from settings. Call [`Self::initialize`] next.
    pub fn new(db: Arc<Db>, cache_dir: PathBuf, enabled: bool, cosmetic: bool) -> AppResult<Self> {
        let exceptions: HashSet<String> = db
            .kv_get(KV_EXCEPTIONS)?
            .and_then(|j| serde_json::from_str(&j).ok())
            .unwrap_or_default();
        Ok(AdBlockEngine {
            engine: RwLock::new(None),
            enabled: AtomicBool::new(enabled),
            cosmetic: AtomicBool::new(cosmetic),
            exceptions: RwLock::new(exceptions),
            private_overrides: RwLock::default(),
            total_blocked: AtomicU64::new(0),
            status: Mutex::new(EngineStatus::default()),
            rebuild_lock: Mutex::new(()),
            update_lock: Mutex::new(()),
            rerun: AtomicBool::new(false),
            catalogue_lock: Mutex::new(()),
            built_from: Mutex::new(String::new()),
            cache_dir,
            db,
            on_change: RwLock::new(None),
        })
    }

    pub fn set_change_hook(&self, hook: ChangeHook) {
        *self.on_change.write().unwrap_or_else(|p| p.into_inner()) = Some(hook);
    }

    fn changed(&self) {
        if let Some(hook) = self
            .on_change
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
        {
            hook();
        }
    }

    /// Loads the cached engine (fast path), rebuilding from cached lists if the cache is stale.
    /// Blocking: call from a background thread. Returns whether any list needs downloading.
    pub fn initialize(&self) -> bool {
        let lists = self.lists();
        let fingerprint = self.fingerprint(&lists);
        let meta =
            std::fs::read_to_string(self.cache_dir.join(ENGINE_META_FILE)).unwrap_or_default();
        let mut loaded = false;
        if meta == fingerprint {
            if let Ok(bytes) = std::fs::read(self.cache_dir.join(ENGINE_FILE)) {
                let mut engine = Engine::default();
                match engine.deserialize(&bytes) {
                    Ok(()) => {
                        // Count like `rebuild`: an enabled list never downloaded adds no rules.
                        let active = lists
                            .iter()
                            .filter(|l| l.enabled && self.list_path(&l.id).exists())
                            .count();
                        self.install(engine, active, fingerprint);
                        loaded = true;
                    }
                    Err(e) => log::warn!("cached ad-block engine rejected, rebuilding: {e:?}"),
                }
            }
        }
        if !loaded {
            if let Err(e) = self.rebuild() {
                self.set_error(Some(e.to_string()));
            }
        }
        let now = now_millis();
        lists
            .iter()
            .any(|l| l.enabled && (!self.list_path(&l.id).exists() || l.is_stale(now)))
    }

    /// Downloads enabled lists (all, or only stale/missing ones) and rebuilds if the engine's
    /// inputs changed. Blocking. Returns at once when another update is already running.
    pub fn update_lists(&self, force: bool) -> AppResult<()> {
        let result = {
            let _running = match self.update_lock.try_lock() {
                Ok(guard) => guard,
                Err(TryLockError::Poisoned(p)) => p.into_inner(),
                Err(TryLockError::WouldBlock) => {
                    self.rerun.store(true, Ordering::SeqCst);
                    return Ok(());
                }
            };
            self.rerun.store(false, Ordering::SeqCst);
            self.update_once(force)
        };
        // A change arrived while this ran (it may have missed the final rebuild): apply it now
        // rather than at the next periodic check.
        if self.rerun.swap(false, Ordering::SeqCst) {
            let again = self.update_lists(false);
            return result.and(again);
        }
        result
    }

    fn update_once(&self, force: bool) -> AppResult<()> {
        let now = now_millis();
        let due: Vec<FilterList> = self
            .lists()
            .into_iter()
            .filter(|l| l.enabled && (force || !self.list_path(&l.id).exists() || l.is_stale(now)))
            .collect();
        // The periodic check usually ends here; a list toggle still needs the rebuild below.
        if due.is_empty() && self.engine_is_current() {
            return Ok(());
        }
        self.set_updating(true);
        let result = (|| {
            let failed = self.fetch(due, now)?;
            if !self.engine_is_current() {
                self.rebuild()?;
            }
            if failed.is_empty() {
                Ok(())
            } else {
                Err(AppError::Network(format!(
                    "could not update: {}",
                    failed.join(", ")
                )))
            }
        })();
        self.set_error(result.as_ref().err().map(ToString::to_string));
        self.set_updating(false);
        result
    }

    /// Downloads `due` into the cache and records the outcome in the catalogue as it is now, not
    /// as it was when the download started. Returns the titles that failed.
    fn fetch(&self, mut due: Vec<FilterList>, now: i64) -> AppResult<Vec<String>> {
        let mut failed = Vec::new();
        for list in &mut due {
            match lists::download(&list.url) {
                Ok(text) => {
                    std::fs::write(self.list_path(&list.id), text)?;
                    list.last_updated = Some(now);
                    list.last_error = None;
                }
                Err(e) => {
                    log::warn!("filter list '{}' update failed: {e}", list.title);
                    list.last_error = Some(e.to_string());
                    failed.push(list.title.clone());
                }
            }
        }
        self.edit_lists(|lists| {
            lists::merge_results(lists, &due);
            // Removed while downloading: drop the file the download just wrote.
            for gone in due.iter().filter(|d| lists.iter().all(|l| l.id != d.id)) {
                let _ = std::fs::remove_file(self.list_path(&gone.id));
            }
            Ok(())
        })?;
        Ok(failed)
    }

    fn engine_is_current(&self) -> bool {
        let current = self.fingerprint(&self.lists());
        *self.built_from.lock().unwrap_or_else(|p| p.into_inner()) == current
    }

    /// Compiles all enabled cached lists plus custom rules into a new engine and swaps it in.
    pub fn rebuild(&self) -> AppResult<()> {
        let _guard = self.rebuild_lock.lock().unwrap_or_else(|p| p.into_inner());
        let lists = self.lists();
        let fingerprint = self.fingerprint(&lists);
        let mut set = FilterSet::new(false);
        let mut active = 0;
        for list in lists.iter().filter(|l| l.enabled) {
            if let Ok(text) = std::fs::read_to_string(self.list_path(&list.id)) {
                set.add_filter_list(text, ParseOptions::default());
                active += 1;
            }
        }
        let custom = self.custom_rules()?;
        set.add_filter_list(custom, ParseOptions::default());
        let engine = Engine::new_with_filter_set(set);
        let bytes = engine.serialize();
        // Cache write failures only cost startup time; protection still applies in memory.
        if let Err(e) = std::fs::write(self.cache_dir.join(ENGINE_FILE), bytes)
            .and_then(|_| std::fs::write(self.cache_dir.join(ENGINE_META_FILE), &fingerprint))
        {
            log::warn!("could not cache compiled ad-block engine: {e}");
        }
        self.install(engine, active, fingerprint);
        Ok(())
    }

    fn install(&self, engine: Engine, active: usize, fingerprint: String) {
        *self.engine.write().unwrap_or_else(|p| p.into_inner()) = Some(Arc::new(engine));
        *self.built_from.lock().unwrap_or_else(|p| p.into_inner()) = fingerprint;
        {
            let mut s = self.status.lock().unwrap_or_else(|p| p.into_inner());
            s.ready = active > 0;
            s.rule_lists_active = active;
        }
        self.changed();
    }

    /// Identifies the inputs of a compiled engine; a mismatch forces a rebuild.
    fn fingerprint(&self, lists: &[FilterList]) -> String {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for l in lists.iter().filter(|l| l.enabled) {
            (&l.id, l.last_updated).hash(&mut h);
        }
        self.custom_rules().unwrap_or_default().hash(&mut h);
        format!("v1-{}-{:x}", env!("CARGO_PKG_VERSION"), h.finish())
    }

    // --- request evaluation (hot path, called on the UI thread per request) ---

    /// Should a subresource request made by the page at `source_url` (in a private tab or not) be
    /// blocked?
    pub fn should_block(
        &self,
        url: &str,
        source_url: &str,
        kind: ResourceKind,
        private: bool,
    ) -> bool {
        if !self.is_active_for(source_url, private) {
            return false;
        }
        let Some(engine) = self
            .engine
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
        else {
            return false;
        };
        let Ok(request) = Request::new(url, source_url, kind.as_adblock_str(), "GET") else {
            return false;
        };
        let blocked = engine.check_network_request(&request).should_block();
        if blocked {
            self.total_blocked.fetch_add(1, Ordering::Relaxed);
        }
        blocked
    }

    /// Hide-selectors for a page that is starting to load, or `None` when cosmetic filtering is off.
    pub fn cosmetic_css(&self, page_url: &str, private: bool) -> Option<CosmeticPlan> {
        if !self.cosmetic.load(Ordering::Relaxed) || !self.is_active_for(page_url, private) {
            return None;
        }
        let engine = self
            .engine
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()?;
        let res = engine.url_cosmetic_resources(page_url);
        Some(CosmeticPlan {
            css: hide_css(res.hide_selectors.iter()),
            generic_allowed: !res.generichide,
            exceptions: res.exceptions,
        })
    }

    /// Generic class/id-based hide rules for the classes and ids present on a page.
    pub fn generic_css(
        &self,
        classes: &[String],
        ids: &[String],
        exceptions: &HashSet<String>,
    ) -> String {
        let Some(engine) = self
            .engine
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
        else {
            return String::new();
        };
        hide_css(
            engine
                .hidden_class_id_selectors(classes, ids, exceptions)
                .iter(),
        )
    }

    fn is_active_for(&self, page_url: &str, private: bool) -> bool {
        if !self.enabled.load(Ordering::Relaxed) {
            return false;
        }
        host_of(page_url).is_none_or(|host| !self.excepted(&host, private))
    }

    /// Protection is off for `host`: a private tab's own choice wins, otherwise the saved one.
    fn excepted(&self, host: &str, private: bool) -> bool {
        let overridden = private
            .then(|| {
                self.private_overrides
                    .read()
                    .unwrap_or_else(|p| p.into_inner())
                    .get(host)
                    .copied()
            })
            .flatten();
        overridden.unwrap_or_else(|| {
            self.exceptions
                .read()
                .unwrap_or_else(|p| p.into_inner())
                .contains(host)
        })
    }

    // --- controls ---

    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
        self.changed();
    }

    pub fn set_cosmetic(&self, enabled: bool) {
        self.cosmetic.store(enabled, Ordering::Relaxed);
    }

    pub fn site_enabled(&self, host: &str, private: bool) -> bool {
        !self.excepted(host, private)
    }

    pub fn set_site_exception(&self, host: &str, excepted: bool) -> AppResult<()> {
        let host = clean_host(host)?;
        let json = {
            let mut set = self.exceptions.write().unwrap_or_else(|p| p.into_inner());
            if excepted {
                set.insert(host);
            } else {
                set.remove(&host);
            }
            serde_json::to_string(&*set).unwrap_or_else(|_| "[]".into())
        };
        self.db.kv_set(KV_EXCEPTIONS, &json)?;
        self.changed();
        Ok(())
    }

    /// A choice made in a private tab: kept in memory for private tabs only, never saved.
    pub fn set_private_site_exception(&self, host: &str, excepted: bool) -> AppResult<()> {
        let host = clean_host(host)?;
        self.private_overrides
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .insert(host, excepted);
        self.changed();
        Ok(())
    }

    /// The last private tab closed: forget every choice made in private tabs.
    pub fn end_private_session(&self) {
        self.private_overrides
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
    }

    pub fn site_exceptions(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .exceptions
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .cloned()
            .collect();
        v.sort();
        v
    }

    pub fn status(&self) -> EngineStatus {
        let mut s = self
            .status
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        s.total_blocked = self.total_blocked.load(Ordering::Relaxed);
        s
    }

    fn set_updating(&self, updating: bool) {
        self.status
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .updating = updating;
        self.changed();
    }

    fn set_error(&self, error: Option<String>) {
        self.status
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .last_error = error;
        self.changed();
    }

    // --- filter list and custom rule management ---

    pub fn lists(&self) -> Vec<FilterList> {
        lists::load(&self.db)
    }

    /// The only way to change the catalogue: load, edit and save under one lock.
    fn edit_lists<T>(
        &self,
        edit: impl FnOnce(&mut Vec<FilterList>) -> AppResult<T>,
    ) -> AppResult<T> {
        let _guard = self
            .catalogue_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let mut lists = self.lists();
        let out = edit(&mut lists)?;
        lists::save(&self.db, &lists)?;
        Ok(out)
    }

    fn list_path(&self, id: &str) -> PathBuf {
        self.cache_dir.join(format!("{id}.txt"))
    }

    pub fn add_list(&self, url: &str, title: &str) -> AppResult<FilterList> {
        let parsed = crate::browser::url::web_url(url).filter(|u| u.scheme() == "https");
        let Some(parsed) = parsed else {
            return Err(AppError::InvalidInput(
                "filter lists must use an https:// URL".into(),
            ));
        };
        let list = FilterList::custom(parsed.as_str(), title);
        self.edit_lists(|lists| {
            if lists.iter().any(|l| l.url == list.url) {
                return Err(AppError::InvalidInput("this list is already added".into()));
            }
            lists.push(list.clone());
            Ok(())
        })?;
        Ok(list)
    }

    pub fn remove_list(&self, id: &str) -> AppResult<()> {
        self.edit_lists(|lists| {
            let before = lists.len();
            lists.retain(|l| l.id != id || l.builtin);
            if lists.len() == before {
                return Err(AppError::InvalidInput(
                    "built-in lists can be disabled but not removed".into(),
                ));
            }
            Ok(())
        })?;
        let _ = std::fs::remove_file(self.list_path(id));
        Ok(())
    }

    pub fn set_list_enabled(&self, id: &str, enabled: bool) -> AppResult<()> {
        self.edit_lists(|lists| {
            let list = lists
                .iter_mut()
                .find(|l| l.id == id)
                .ok_or_else(|| AppError::NotFound("filter list".into()))?;
            list.enabled = enabled;
            Ok(())
        })
    }

    pub fn custom_rules(&self) -> AppResult<String> {
        Ok(self.db.kv_get(KV_CUSTOM_RULES)?.unwrap_or_default())
    }

    pub fn set_custom_rules(&self, rules: &str) -> AppResult<()> {
        if rules.len() > 512 * 1024 {
            return Err(AppError::InvalidInput(
                "custom rules are limited to 512 KB".into(),
            ));
        }
        self.db.kv_set(KV_CUSTOM_RULES, rules)
    }
}

/// One rule per selector: a single invalid selector would otherwise void a whole selector list.
fn hide_css<'a>(selectors: impl Iterator<Item = &'a String>) -> String {
    let mut css = String::new();
    for s in selectors {
        // `;` or a leading `@` would let a list smuggle in at-rules such as `@import url(...)`;
        // a comment opener/closer or a trailing `\` would swallow the rule that follows.
        if s.contains(['{', '}', '<', ';'])
            || s.contains("/*")
            || s.contains("*/")
            || s.ends_with('\\')
            || s.trim_start().starts_with('@')
            || !balanced(s)
        {
            continue;
        }
        css.push_str(s);
        css.push_str("{display:none!important}\n");
    }
    css
}

/// An unclosed `(`, `[` or string would swallow every rule after it (`##div:not(.x`).
fn balanced(s: &str) -> bool {
    let count = |c| s.matches(c).count();
    count('(') == count(')')
        && count('[') == count(']')
        && count('"') % 2 == 0
        && count('\'') % 2 == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine_with(rules: &str) -> (AdBlockEngine, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "veyscape-adblock-{}-{}",
            now_millis(),
            rand_suffix()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db = Arc::new(Db::open_in_memory().unwrap());
        // Disable built-in lists so tests never touch the network or real list files.
        let mut lists = lists::load(&db);
        lists.iter_mut().for_each(|l| l.enabled = false);
        lists::save(&db, &lists).unwrap();
        let e = AdBlockEngine::new(db, dir.clone(), true, true).unwrap();
        e.set_custom_rules(rules).unwrap();
        e.rebuild().unwrap();
        (e, dir)
    }

    fn rand_suffix() -> u64 {
        use std::hash::{BuildHasher, RandomState};
        RandomState::new().hash_one(std::thread::current().id())
    }

    const RULES: &str = "||ads.example^\n||tracker.example^$third-party\n@@||ads.example/allowed.js\nnews.example##.sponsored\n##.ad-banner";

    #[test]
    fn blocks_matching_requests_and_honours_exception_rules() {
        let (e, dir) = engine_with(RULES);
        let page = "https://news.example/article";
        assert!(e.should_block(
            "https://ads.example/banner.js",
            page,
            ResourceKind::Script,
            false
        ));
        assert!(!e.should_block(
            "https://ads.example/allowed.js",
            page,
            ResourceKind::Script,
            false
        ));
        assert!(e.should_block(
            "https://tracker.example/pixel.gif",
            page,
            ResourceKind::Image,
            false
        ));
        assert!(!e.should_block(
            "https://tracker.example/app.js",
            "https://tracker.example/",
            ResourceKind::Script,
            false
        ));
        assert!(!e.should_block(
            "https://cdn.example/lib.js",
            page,
            ResourceKind::Script,
            false
        ));
        assert_eq!(e.status().total_blocked, 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn global_toggle_and_site_exceptions_disable_blocking() {
        let (e, dir) = engine_with(RULES);
        let page = "https://news.example/";
        e.set_site_exception("news.example", true).unwrap();
        assert!(!e.should_block(
            "https://ads.example/a.js",
            page,
            ResourceKind::Script,
            false
        ));
        assert!(e.should_block(
            "https://ads.example/a.js",
            "https://other.example/",
            ResourceKind::Script,
            false
        ));
        e.set_site_exception("news.example", false).unwrap();
        // Private tabs: their own choices, in memory only, gone when the private session ends.
        e.set_private_site_exception("news.example", true).unwrap();
        assert!(!e.should_block("https://ads.example/a.js", page, ResourceKind::Script, true));
        assert!(e.should_block(
            "https://ads.example/a.js",
            page,
            ResourceKind::Script,
            false
        ));
        assert!(e.site_exceptions().is_empty(), "nothing is saved");
        e.end_private_session();
        assert!(e.should_block("https://ads.example/a.js", page, ResourceKind::Script, true));
        // A private "re-enable" wins over a saved exception, for private tabs only.
        e.set_site_exception("news.example", true).unwrap();
        e.set_private_site_exception("news.example", false).unwrap();
        assert!(e.should_block("https://ads.example/a.js", page, ResourceKind::Script, true));
        assert!(!e.should_block(
            "https://ads.example/a.js",
            page,
            ResourceKind::Script,
            false
        ));
        e.set_site_exception("news.example", false).unwrap();
        e.set_enabled(false);
        assert!(!e.should_block(
            "https://ads.example/a.js",
            page,
            ResourceKind::Script,
            false
        ));
        assert!(e.set_site_exception("bad/host", true).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cosmetic_rules_produce_hide_css() {
        let (e, dir) = engine_with(RULES);
        let plan = e.cosmetic_css("https://news.example/", false).unwrap();
        assert!(plan.css.contains(".sponsored{display:none!important}"));
        let generic = e.generic_css(
            &["ad-banner".into(), "content".into()],
            &[],
            &plan.exceptions,
        );
        assert!(generic.contains(".ad-banner"));
        assert!(!generic.contains(".content"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cached_engine_is_reused_on_initialize() {
        let (e, dir) = engine_with(RULES);
        let db = e.db.clone();
        drop(e);
        let fresh = AdBlockEngine::new(db, dir.clone(), true, true).unwrap();
        fresh.initialize();
        assert!(fresh.should_block(
            "https://ads.example/x.js",
            "https://a.example/",
            ResourceKind::Script,
            false
        ));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn update_without_stale_lists_rebuilds_only_when_inputs_changed() {
        let (e, dir) = engine_with(RULES);
        let installed = || e.engine.read().unwrap().clone().unwrap();
        let before = installed();
        e.update_lists(false).unwrap();
        assert!(
            Arc::ptr_eq(&before, &installed()),
            "no-op update keeps the engine"
        );
        // A toggle or rule change still reaches the engine through the same call.
        e.set_custom_rules("||other.example^").unwrap();
        e.update_lists(false).unwrap();
        assert!(e.should_block(
            "https://other.example/x.js",
            "https://a.example/",
            ResourceKind::Script,
            false
        ));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn initialize_counts_only_downloaded_lists() {
        let (e, dir) = engine_with(RULES);
        e.set_list_enabled("easylist", true).unwrap();
        e.rebuild().unwrap();
        let fresh = AdBlockEngine::new(e.db.clone(), dir.clone(), true, true).unwrap();
        assert!(
            fresh.initialize(),
            "the missing list still needs downloading"
        );
        assert_eq!(fresh.status().rule_lists_active, 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn list_management_validates_input() {
        let (e, dir) = engine_with("");
        assert!(e.add_list("http://insecure.example/list.txt", "x").is_err());
        let l = e
            .add_list("https://lists.example/list.txt", "Mine")
            .unwrap();
        assert!(e.add_list("https://lists.example/list.txt", "dup").is_err());
        e.set_list_enabled(&l.id, false).unwrap();
        e.remove_list(&l.id).unwrap();
        assert!(
            e.remove_list("easylist").is_err(),
            "built-ins cannot be removed"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn hide_css_skips_unsafe_selectors() {
        let sels = [
            ".a".to_string(),
            "x{}".to_string(),
            "@import url(https://evil.example/x.css);#x".to_string(),
            "#x;color:red".to_string(),
            " @namespace x".to_string(),
            "#y /* x".to_string(),
            "x */ #z".to_string(),
            "#w\\".to_string(),
            "div:not(.x".to_string(),
            "a[href".to_string(),
            "a[title=\"x]".to_string(),
            "a[title='x]".to_string(),
            "a[title=\"x\"]:not(.y)".to_string(),
        ];
        assert_eq!(
            hide_css(sels.iter()),
            ".a{display:none!important}\na[title=\"x\"]:not(.y){display:none!important}\n"
        );
    }
}
