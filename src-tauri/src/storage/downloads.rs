use super::Db;
use crate::errors::AppResult;
use rusqlite::params;

/// Persisted download row. Private-tab downloads are never written here.
#[derive(Clone, Debug, PartialEq)]
pub struct DownloadRecord {
    pub id: i64,
    pub url: String,
    pub path: String,
    pub state: String,
    pub received: i64,
    pub total: Option<i64>,
    pub started_at: i64,
}

impl Db {
    pub fn download_upsert(&self, d: &DownloadRecord) -> AppResult<()> {
        self.conn().execute(
            "INSERT INTO downloads (id, url, path, state, received, total, started_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET path = excluded.path, state = excluded.state,
               received = excluded.received, total = excluded.total",
            params![d.id, d.url, d.path, d.state, d.received, d.total, d.started_at],
        )?;
        Ok(())
    }

    pub fn downloads_list(&self, limit: u32) -> AppResult<Vec<DownloadRecord>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, url, path, state, received, total, started_at FROM downloads ORDER BY started_at DESC, id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit], |r| {
            Ok(DownloadRecord {
                id: r.get(0)?,
                url: r.get(1)?,
                path: r.get(2)?,
                state: r.get(3)?,
                received: r.get(4)?,
                total: r.get(5)?,
                started_at: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn downloads_max_id(&self) -> AppResult<i64> {
        Ok(self
            .conn()
            .query_row("SELECT COALESCE(MAX(id), 0) FROM downloads", [], |r| {
                r.get(0)
            })?)
    }

    pub fn download_remove(&self, id: i64) -> AppResult<()> {
        self.conn()
            .execute("DELETE FROM downloads WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Removes finished entries started at or after `since` (Unix ms, 0 = all) from the list
    /// (files on disk are untouched).
    pub fn downloads_clear_finished(&self, since: i64) -> AppResult<()> {
        self.conn().execute(
            "DELETE FROM downloads WHERE state != 'inProgress' AND started_at >= ?1",
            [since],
        )?;
        Ok(())
    }

    /// Downloads left `inProgress` by a crash or forced exit cannot be resumed across sessions.
    pub fn downloads_mark_stale_interrupted(&self) -> AppResult<()> {
        self.conn().execute(
            "UPDATE downloads SET state = 'interrupted' WHERE state = 'inProgress'",
            [],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_and_stale_cleanup() {
        let db = Db::open_in_memory().unwrap();
        let mut d = DownloadRecord {
            id: 7,
            url: "https://a.example/f.zip".into(),
            path: "C:\\f.zip".into(),
            state: "inProgress".into(),
            received: 1,
            total: Some(10),
            started_at: 1,
        };
        db.download_upsert(&d).unwrap();
        d.received = 5;
        db.download_upsert(&d).unwrap();
        assert_eq!(db.downloads_list(10).unwrap()[0].received, 5);
        assert_eq!(db.downloads_max_id().unwrap(), 7);
        db.downloads_mark_stale_interrupted().unwrap();
        assert_eq!(db.downloads_list(10).unwrap()[0].state, "interrupted");
        db.downloads_clear_finished(0).unwrap();
        assert!(db.downloads_list(10).unwrap().is_empty());
    }

    #[test]
    fn clear_finished_respects_the_time_range() {
        let db = Db::open_in_memory().unwrap();
        let row = |id, state: &str, started_at| DownloadRecord {
            id,
            url: "https://a.example/f.zip".into(),
            path: "C:\\f.zip".into(),
            state: state.into(),
            received: 0,
            total: None,
            started_at,
        };
        db.download_upsert(&row(1, "completed", 100)).unwrap();
        db.download_upsert(&row(2, "completed", 500)).unwrap();
        db.download_upsert(&row(3, "inProgress", 500)).unwrap();
        db.downloads_clear_finished(200).unwrap();
        let ids: Vec<i64> = db
            .downloads_list(10)
            .unwrap()
            .iter()
            .map(|d| d.id)
            .collect();
        assert_eq!(ids, [3, 1], "older and running downloads stay");
    }
}
