//! Filter-list catalogue (persisted in the `kv` table) and downloading.

use crate::errors::{AppError, AppResult};
use crate::storage::Db;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::time::Duration;

pub const DEFAULT_UPDATE_INTERVAL_MS: i64 = 4 * 24 * 60 * 60 * 1000;
const KV_LISTS: &str = "adblock.lists";
const MAX_LIST_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FilterList {
    pub id: String,
    pub title: String,
    pub url: String,
    pub enabled: bool,
    pub builtin: bool,
    pub license: String,
    pub last_updated: Option<i64>,
    pub last_error: Option<String>,
}

impl FilterList {
    fn builtin(id: &str, title: &str, url: &str, enabled: bool, license: &str) -> Self {
        FilterList {
            id: id.into(),
            title: title.into(),
            url: url.into(),
            enabled,
            builtin: true,
            license: license.into(),
            last_updated: None,
            last_error: None,
        }
    }

    pub fn custom(url: &str, title: &str) -> Self {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        url.hash(&mut h);
        let title = if title.trim().is_empty() {
            url
        } else {
            title.trim()
        };
        FilterList {
            id: format!("custom-{:x}", h.finish()),
            title: title.chars().take(80).collect(),
            url: url.into(),
            enabled: true,
            builtin: false,
            license: "See list source".into(),
            last_updated: None,
            last_error: None,
        }
    }

    pub fn is_stale(&self, now: i64) -> bool {
        self.last_updated
            .is_none_or(|t| now - t > DEFAULT_UPDATE_INTERVAL_MS)
    }
}

/// Built-in catalogue. Lists are downloaded from their maintainers, never bundled or proxied.
fn defaults() -> Vec<FilterList> {
    vec![
        FilterList::builtin("easylist", "EasyList (ads)", "https://easylist.to/easylist/easylist.txt", true, "GPL-3.0 / CC BY-SA 3.0"),
        FilterList::builtin("easyprivacy", "EasyPrivacy (trackers)", "https://easylist.to/easylist/easyprivacy.txt", true, "GPL-3.0 / CC BY-SA 3.0"),
        FilterList::builtin(
            "urlhaus",
            "Malicious URL Blocklist (URLhaus)",
            "https://malware-filter.gitlab.io/malware-filter/urlhaus-filter.txt",
            // ~13 MB: roughly quadruples engine memory, so opt-in.
            false,
            "CC0 / MIT",
        ),
        FilterList::builtin(
            "peterlowe",
            "Peter Lowe's ad and tracking server list",
            "https://pgl.yoyo.org/adservers/serverlist.php?hostformat=adblockplus&showintro=0&mimetype=plaintext",
            false,
            "See pgl.yoyo.org",
        ),
        FilterList::builtin("cookies", "EasyList Cookie List (cookie banners)", "https://secure.fanboy.co.nz/fanboy-cookiemonster.txt", false, "GPL-3.0 / CC BY-SA 3.0"),
        FilterList::builtin("annoyances", "Fanboy's Annoyance List", "https://secure.fanboy.co.nz/fanboy-annoyance.txt", false, "GPL-3.0 / CC BY-SA 3.0"),
    ]
}

/// Stored catalogue merged with the current built-ins (new built-ins appear after upgrades).
pub fn load(db: &Db) -> Vec<FilterList> {
    let stored: Vec<FilterList> = db
        .kv_get(KV_LISTS)
        .ok()
        .flatten()
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default();
    let mut lists: Vec<FilterList> = defaults()
        .into_iter()
        .map(|d| match stored.iter().find(|s| s.id == d.id) {
            Some(s) => FilterList {
                enabled: s.enabled,
                last_updated: s.last_updated,
                last_error: s.last_error.clone(),
                ..d
            },
            None => d,
        })
        .collect();
    lists.extend(stored.into_iter().filter(|s| !s.builtin));
    lists
}

pub fn save(db: &Db, lists: &[FilterList]) -> AppResult<()> {
    let json = serde_json::to_string(lists).map_err(|e| AppError::InvalidInput(e.to_string()))?;
    db.kv_set(KV_LISTS, &json)
}

/// Copies download results onto the current catalogue by id. Everything else (enabled flags,
/// added or removed lists) belongs to the user and may have changed during the download.
pub fn merge_results(current: &mut [FilterList], fetched: &[FilterList]) {
    for list in current {
        if let Some(f) = fetched.iter().find(|f| f.id == list.id) {
            list.last_updated = f.last_updated;
            list.last_error = f.last_error.clone();
        }
    }
}

/// Fetches a list over HTTPS only, redirects included, with certificate verification
/// (ureq/rustls defaults; never disabled).
pub fn download(url: &str) -> AppResult<String> {
    let agent = ureq::Agent::config_builder()
        .https_only(true)
        // A stalled server must not keep the update, and its "updating" status, alive forever.
        // Generous: the largest list (~13 MB) must still arrive over a slow connection.
        .timeout_global(Some(Duration::from_secs(300)))
        .build()
        .new_agent();
    let response = agent
        .get(url)
        .header("User-Agent", "Veyscape filter updater")
        .call()
        .map_err(|e| AppError::Network(e.to_string()))?;
    let text = read_capped(response.into_body().into_reader(), MAX_LIST_BYTES)?;
    // Guard against captive portals and error pages being parsed as rules.
    if text.trim_start().starts_with('<') || text.lines().count() < 3 {
        return Err(AppError::Network("response is not a filter list".into()));
    }
    Ok(text)
}

/// Fails instead of truncating: a list cut mid-line can turn `||adservice.example^` into `||ad`.
fn read_capped(reader: impl Read, max: u64) -> AppResult<String> {
    let mut bytes = Vec::new();
    reader
        .take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| AppError::Network(e.to_string()))?;
    if bytes.len() as u64 > max {
        return Err(AppError::Network("filter list is too large".into()));
    }
    String::from_utf8(bytes).map_err(|_| AppError::Network("filter list is not UTF-8".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_enable_ads_and_trackers() {
        let db = Db::open_in_memory().unwrap();
        let lists = load(&db);
        let enabled: Vec<_> = lists
            .iter()
            .filter(|l| l.enabled)
            .map(|l| l.id.as_str())
            .collect();
        assert_eq!(enabled, ["easylist", "easyprivacy"]);
        assert!(lists.iter().all(|l| l.url.starts_with("https://")));
    }

    #[test]
    fn stored_state_survives_and_custom_lists_are_kept() {
        let db = Db::open_in_memory().unwrap();
        let mut lists = load(&db);
        lists[0].enabled = false;
        lists.push(FilterList::custom("https://x.example/l.txt", "X"));
        save(&db, &lists).unwrap();
        let again = load(&db);
        assert!(!again[0].enabled);
        assert!(again.iter().any(|l| l.title == "X"));
    }

    #[test]
    fn staleness() {
        let mut l = FilterList::custom("https://x.example/", "");
        assert!(l.is_stale(0));
        l.last_updated = Some(1_000);
        assert!(!l.is_stale(2_000));
        assert!(l.is_stale(1_000 + DEFAULT_UPDATE_INTERVAL_MS + 1));
    }

    #[test]
    fn merge_keeps_changes_made_during_download() {
        let mut fetched = defaults();
        fetched.push(FilterList::custom("https://x.example/l.txt", "X"));
        fetched.iter_mut().for_each(|l| l.last_updated = Some(7));
        // Meanwhile the user disabled EasyPrivacy and removed the custom list.
        let mut current = defaults();
        current[1].enabled = false;
        merge_results(&mut current, &fetched);
        assert!(!current[1].enabled);
        assert_eq!(current[1].last_updated, Some(7));
        assert!(
            current.iter().all(|l| l.builtin),
            "removed list stays removed"
        );
    }

    #[test]
    fn oversized_lists_are_rejected_not_truncated() {
        assert_eq!(read_capped(&b"abc"[..], 3).unwrap(), "abc");
        assert!(read_capped(&b"abcd"[..], 3).is_err());
    }
}
