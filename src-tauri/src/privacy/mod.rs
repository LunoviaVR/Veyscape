//! Browsing-data deletion. Local records (history, download list, permissions) are deleted here;
//! engine data (cookies, site storage, cache) is deleted by the platform engine.

use crate::errors::AppResult;
use crate::platform::ClearDataKinds;
use crate::storage::{now_millis, Db};
use serde::Deserialize;

#[derive(Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TimeRange {
    LastHour,
    LastDay,
    LastWeek,
    #[default]
    AllTime,
}

impl TimeRange {
    /// Unix-ms lower bound; `0` means everything.
    pub fn since(self, now: i64) -> i64 {
        const HOUR: i64 = 60 * 60 * 1000;
        match self {
            TimeRange::LastHour => now - HOUR,
            TimeRange::LastDay => now - 24 * HOUR,
            TimeRange::LastWeek => now - 7 * 24 * HOUR,
            TimeRange::AllTime => 0,
        }
    }
}

#[derive(Deserialize, Clone, Copy, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ClearDataRequest {
    pub history: bool,
    pub downloads: bool,
    pub cookies: bool,
    pub cache: bool,
    pub permissions: bool,
    /// Applies to history and the download list (also the engine's own records of both); cookies
    /// and cache are always cleared entirely.
    pub range: TimeRange,
}

impl ClearDataRequest {
    pub fn engine_kinds(&self) -> ClearDataKinds {
        ClearDataKinds {
            cookies: self.cookies,
            cache: self.cache,
            history: self.history,
            downloads: self.downloads,
            since_ms: self.range.since(now_millis()),
        }
    }
}

pub fn clear_local(db: &Db, req: &ClearDataRequest) -> AppResult<()> {
    let since = req.range.since(now_millis());
    if req.history {
        db.history_clear(since)?;
    }
    if req.downloads {
        db.downloads_clear_finished(since)?;
    }
    if req.permissions {
        db.permissions_clear()?;
    }
    // Deleted rows must not linger in the write-ahead log.
    db.checkpoint()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clears_only_requested_categories() {
        let db = Db::open_in_memory().unwrap();
        db.history_record_visit("https://a.example/", "A").unwrap();
        db.permission_set("https://a.example", "camera", Some("allow"))
            .unwrap();
        clear_local(
            &db,
            &ClearDataRequest {
                history: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(db.history_list("", 10, 0).unwrap().is_empty());
        assert_eq!(db.permissions_list().unwrap().len(), 1);
        clear_local(
            &db,
            &ClearDataRequest {
                permissions: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(db.permissions_list().unwrap().is_empty());
    }

    #[test]
    fn time_ranges() {
        assert_eq!(TimeRange::AllTime.since(10_000_000), 0);
        assert_eq!(
            TimeRange::LastHour.since(10_000_000),
            10_000_000 - 3_600_000
        );
    }
}
