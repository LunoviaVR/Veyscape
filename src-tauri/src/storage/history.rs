use super::{now_millis, Db};
use crate::errors::AppResult;
use rusqlite::params;
use serde::Serialize;

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: i64,
    pub url: String,
    pub title: String,
    pub visit_count: i64,
    pub last_visit: i64,
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<HistoryEntry> {
    Ok(HistoryEntry {
        id: r.get(0)?,
        url: r.get(1)?,
        title: r.get(2)?,
        visit_count: r.get(3)?,
        last_visit: r.get(4)?,
    })
}

/// An address-bar suggestion. `history_id` is set for history rows (they can be removed).
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    pub url: String,
    pub title: String,
    pub bookmarked: bool,
    pub history_id: Option<i64>,
}

/// Escapes LIKE wildcards so user input is matched literally (paired with `ESCAPE '!'`).
fn escape_like(query: &str) -> String {
    query
        .replace('!', "!!")
        .replace('%', "!%")
        .replace('_', "!_")
}

fn like_pattern(query: &str) -> String {
    format!("%{}%", escape_like(query))
}

impl Db {
    pub fn history_record_visit(&self, url: &str, title: &str) -> AppResult<()> {
        self.conn().execute(
            "INSERT INTO history (url, title, visit_count, last_visit) VALUES (?1, ?2, 1, ?3)
             ON CONFLICT(url) DO UPDATE SET visit_count = visit_count + 1, last_visit = excluded.last_visit,
               title = CASE WHEN excluded.title = '' THEN history.title ELSE excluded.title END",
            params![url, title, now_millis()],
        )?;
        Ok(())
    }

    pub fn history_update_title(&self, url: &str, title: &str) -> AppResult<()> {
        self.conn().execute(
            "UPDATE history SET title = ?1 WHERE url = ?2",
            params![title, url],
        )?;
        Ok(())
    }

    pub fn history_list(
        &self,
        query: &str,
        limit: u32,
        offset: u32,
    ) -> AppResult<Vec<HistoryEntry>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, url, title, visit_count, last_visit FROM history
             WHERE ?1 = '' OR url LIKE ?2 ESCAPE '!' OR title LIKE ?2 ESCAPE '!'
             ORDER BY last_visit DESC LIMIT ?3 OFFSET ?4",
        )?;
        let rows = stmt.query_map(params![query, like_pattern(query), limit, offset], row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn history_top_sites(&self, limit: u32) -> AppResult<Vec<HistoryEntry>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, url, title, visit_count, last_visit FROM history ORDER BY visit_count DESC, last_visit DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit], row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Address bar suggestions: bookmarks first, then frequent history. Local only.
    /// Returns `(url, title, is_bookmark)`.
    pub fn suggest(&self, query: &str, limit: u32) -> AppResult<Vec<Suggestion>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT url, title, 1 AS bm, 0 AS score, NULL FROM bookmarks WHERE kind = 'bookmark' AND (url LIKE ?1 ESCAPE '!' OR title LIKE ?1 ESCAPE '!') GROUP BY url
             UNION ALL
             SELECT url, title, 0, visit_count, id FROM history WHERE (url LIKE ?1 ESCAPE '!' OR title LIKE ?1 ESCAPE '!')
               AND url NOT IN (SELECT url FROM bookmarks WHERE kind = 'bookmark' AND (url LIKE ?1 ESCAPE '!' OR title LIKE ?1 ESCAPE '!'))
             ORDER BY bm DESC, score DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![like_pattern(query), limit], |r| {
            Ok(Suggestion {
                url: r.get(0)?,
                title: r.get(1)?,
                bookmarked: r.get::<_, i64>(2)? == 1,
                history_id: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// History and bookmark URLs that start with `typed` after `http(s)://` and an optional
    /// `www.` (or with `typed` itself when it has a scheme), with their visit counts.
    pub fn completion_candidates(&self, typed: &str) -> AppResult<Vec<(String, i64)>> {
        let t = escape_like(typed);
        let patterns: [String; 4] = if typed.contains("://") {
            std::array::from_fn(|_| format!("{t}%"))
        } else {
            [
                format!("http://{t}%"),
                format!("https://{t}%"),
                format!("http://www.{t}%"),
                format!("https://www.{t}%"),
            ]
        };
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT url, visit_count FROM history
               WHERE url LIKE ?1 ESCAPE '!' OR url LIKE ?2 ESCAPE '!' OR url LIKE ?3 ESCAPE '!' OR url LIKE ?4 ESCAPE '!'
             UNION ALL
             SELECT url, 1 FROM bookmarks WHERE kind = 'bookmark'
               AND (url LIKE ?1 ESCAPE '!' OR url LIKE ?2 ESCAPE '!' OR url LIKE ?3 ESCAPE '!' OR url LIKE ?4 ESCAPE '!')
             ORDER BY 2 DESC LIMIT 50",
        )?;
        let rows = stmt.query_map(
            params![patterns[0], patterns[1], patterns[2], patterns[3]],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn history_delete(&self, id: i64) -> AppResult<()> {
        self.conn()
            .execute("DELETE FROM history WHERE id = ?1", [id])?;
        self.checkpoint()
    }

    /// Deletes visits newer than `since` (unix ms); `0` clears everything.
    pub fn history_clear(&self, since: i64) -> AppResult<()> {
        self.conn()
            .execute("DELETE FROM history WHERE last_visit >= ?1", [since])?;
        Ok(())
    }

    /// Drops visits older than `before` (unix ms).
    pub(super) fn history_prune(&self, before: i64) -> AppResult<()> {
        self.conn()
            .execute("DELETE FROM history WHERE last_visit < ?1", [before])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visits_aggregate_and_search_matches_literally() {
        let db = Db::open_in_memory().unwrap();
        db.history_record_visit("https://a.example/", "Alpha")
            .unwrap();
        db.history_record_visit("https://a.example/", "").unwrap();
        db.history_record_visit("https://b.example/100%", "Beta")
            .unwrap();
        let all = db.history_list("", 10, 0).unwrap();
        assert_eq!(all.len(), 2);
        let a = all.iter().find(|e| e.url == "https://a.example/").unwrap();
        assert_eq!((a.visit_count, a.title.as_str()), (2, "Alpha"));
        assert_eq!(db.history_list("100%", 10, 0).unwrap().len(), 1);
        assert_eq!(
            db.history_list("%", 10, 0).unwrap().len(),
            1,
            "% must not act as a wildcard"
        );
        assert_eq!(
            db.history_top_sites(1).unwrap()[0].url,
            "https://a.example/"
        );
    }

    #[test]
    fn suggestions_rank_bookmarks_first_without_duplicates() {
        let db = Db::open_in_memory().unwrap();
        db.history_record_visit("https://news.example/", "News")
            .unwrap();
        db.history_record_visit("https://docs.example/", "Docs")
            .unwrap();
        db.bookmark_toggle("https://docs.example/", "Docs", None)
            .unwrap();
        let s = db.suggest("example", 8).unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(
            (s[0].url.as_str(), s[0].bookmarked, s[0].history_id),
            ("https://docs.example/", true, None)
        );
        assert!(!s[1].bookmarked && s[1].history_id.is_some());
    }

    #[test]
    fn history_matches_are_kept_when_the_bookmark_itself_does_not_match() {
        let db = Db::open_in_memory().unwrap();
        db.history_record_visit("https://mail.example/", "Inbox (3) - Example Mail")
            .unwrap();
        db.bookmark_toggle("https://mail.example/", "Mail", None)
            .unwrap();
        let s = db.suggest("inbox", 8).unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].url, "https://mail.example/");
        assert!(s[0].history_id.is_some());
    }

    #[test]
    fn completion_candidates_match_after_scheme_and_www() {
        let db = Db::open_in_memory().unwrap();
        db.history_record_visit("https://www.github.com/login", "")
            .unwrap();
        db.history_record_visit("https://gist.github.com/", "")
            .unwrap();
        db.history_record_visit("https://example.com/?q=git", "")
            .unwrap();
        db.history_record_visit("https://gxt.example/", "").unwrap();
        let urls = |q: &str| -> Vec<String> {
            let mut v: Vec<_> = db
                .completion_candidates(q)
                .unwrap()
                .into_iter()
                .map(|(u, _)| u)
                .collect();
            v.sort();
            v
        };
        assert_eq!(
            urls("gi"),
            ["https://gist.github.com/", "https://www.github.com/login"]
        );
        assert_eq!(urls("https://gis"), ["https://gist.github.com/"]);
        assert!(urls("g_").is_empty(), "_ must not act as a wildcard");
    }

    #[test]
    fn clear_since_keeps_older_entries() {
        let db = Db::open_in_memory().unwrap();
        db.history_record_visit("https://old.example/", "Old")
            .unwrap();
        db.conn()
            .execute("UPDATE history SET last_visit = 10", [])
            .unwrap();
        db.history_record_visit("https://new.example/", "New")
            .unwrap();
        db.history_clear(1000).unwrap();
        let left = db.history_list("", 10, 0).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].url, "https://old.example/");
        db.history_prune(11).unwrap();
        assert!(db.history_list("", 10, 0).unwrap().is_empty());
    }
}
