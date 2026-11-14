//! SQLite persistence. One connection behind a mutex; every query is short.
//! Each table has its own `impl Db` block in a sibling file.

mod bookmarks;
mod downloads;
mod history;
mod kv;
mod logins;
mod permissions;

pub use bookmarks::{Bookmark, ImportNode, ImportSummary, FOLDER_ICONS};
pub use downloads::DownloadRecord;
pub use history::{HistoryEntry, Suggestion};
pub use logins::Login;
pub use permissions::SitePermission;

use crate::errors::AppResult;
use rusqlite::Connection;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

/// Ordered schema migrations. Append only; never edit a released migration.
const MIGRATIONS: &[&str] = &[
    // 1: initial schema
    "CREATE TABLE kv (key TEXT PRIMARY KEY, value TEXT NOT NULL);
     CREATE TABLE bookmarks (id INTEGER PRIMARY KEY, url TEXT NOT NULL UNIQUE, title TEXT NOT NULL, created_at INTEGER NOT NULL);
     CREATE TABLE history (id INTEGER PRIMARY KEY, url TEXT NOT NULL UNIQUE, title TEXT NOT NULL,
                           visit_count INTEGER NOT NULL DEFAULT 1, last_visit INTEGER NOT NULL);
     CREATE INDEX history_last_visit ON history(last_visit DESC);
     CREATE TABLE site_permissions (origin TEXT NOT NULL, kind TEXT NOT NULL, state TEXT NOT NULL, PRIMARY KEY (origin, kind));
     CREATE TABLE downloads (id INTEGER PRIMARY KEY, url TEXT NOT NULL, path TEXT NOT NULL, state TEXT NOT NULL,
                             received INTEGER NOT NULL, total INTEGER, started_at INTEGER NOT NULL);",
    // 2: bookmark folders (tree), folder icons and ordering; URLs may repeat across folders.
    "CREATE TABLE bookmarks_v2 (id INTEGER PRIMARY KEY, parent_id INTEGER, kind TEXT NOT NULL,
                                url TEXT, title TEXT NOT NULL, icon TEXT, position INTEGER NOT NULL,
                                created_at INTEGER NOT NULL);
     INSERT INTO bookmarks_v2 (id, parent_id, kind, url, title, icon, position, created_at)
       SELECT id, NULL, 'bookmark', url, title, NULL, id, created_at FROM bookmarks;
     DROP TABLE bookmarks;
     ALTER TABLE bookmarks_v2 RENAME TO bookmarks;
     CREATE INDEX bookmarks_url ON bookmarks(url);
     CREATE INDEX bookmarks_parent ON bookmarks(parent_id, position);",
    // 3: folder colours (preset name or #rrggbb, same values as tab groups).
    "ALTER TABLE bookmarks ADD COLUMN color TEXT;",
    // 4: site icon of each bookmark (small PNG data URL, learned from pages the user loads).
    "ALTER TABLE bookmarks ADD COLUMN favicon TEXT;",
    // 5: saved logins (password = DPAPI-encrypted blob) and sites never to offer saving for.
    "CREATE TABLE logins (id INTEGER PRIMARY KEY, origin TEXT NOT NULL, username TEXT NOT NULL,
                          password BLOB NOT NULL, created_at INTEGER NOT NULL,
                          last_used_at INTEGER NOT NULL, UNIQUE (origin, username));
     CREATE TABLE login_never (origin TEXT PRIMARY KEY);",
];

pub struct Db(Mutex<Connection>);

/// History older than a year is dropped at startup so the table (and every scan) stays bounded.
const HISTORY_RETENTION_MS: i64 = 365 * 24 * 60 * 60 * 1000;

impl Db {
    pub fn open(path: &Path) -> AppResult<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        // Deleted rows (history, logins) are zeroed instead of lingering in free pages.
        conn.pragma_update(None, "secure_delete", "ON")?;
        let db = Self::init(conn)?;
        // The `history_last_visit` index makes this a range delete, cheap enough at startup.
        if let Err(e) = db.history_prune(now_millis() - HISTORY_RETENTION_MS) {
            log::warn!("history prune failed: {e}");
        }
        Ok(db)
    }

    pub fn open_in_memory() -> AppResult<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> AppResult<Self> {
        migrate(&mut conn)?;
        Ok(Db(Mutex::new(conn)))
    }

    /// Moves the WAL back into the main file and empties it, so deleted rows do not survive in
    /// the `-wal` file. A busy checkpoint (another reader) is not an error; it is only logged.
    pub(crate) fn checkpoint(&self) -> AppResult<()> {
        let busy: i64 = self
            .conn()
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))?;
        if busy != 0 {
            log::info!("WAL checkpoint after a deletion was busy; it completes later");
        }
        Ok(())
    }

    pub(crate) fn conn(&self) -> MutexGuard<'_, Connection> {
        // A poisoned lock only means another thread panicked mid-query; SQLite itself stays consistent.
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }
}

fn migrate(conn: &mut Connection) -> AppResult<()> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (i + 1) as i64)?;
        tx.commit()?;
    }
    Ok(())
}

pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_are_idempotent() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        migrate(&mut conn).unwrap();
        let v: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(v as usize, MIGRATIONS.len());
    }

    #[test]
    fn data_persists_across_reopen() {
        let dir = std::env::temp_dir().join(format!("veyscape-test-{}", now_millis()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("db.sqlite3");
        {
            let db = Db::open(&path).unwrap();
            db.bookmark_toggle("https://example.com/", "Example", None)
                .unwrap();
        }
        let db = Db::open(&path).unwrap();
        assert_eq!(db.bookmarks_list().unwrap().len(), 1);
        drop(db);
        let _ = std::fs::remove_dir_all(dir);
    }
}
