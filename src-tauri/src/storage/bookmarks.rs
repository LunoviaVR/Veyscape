//! Bookmark tree: folders and bookmarks, ordered by `position` within a parent.
//! Items with `parent_id = NULL` are shown on the bookmark bar.

use super::{now_millis, Db};
use crate::bookmarks::favicon;
use crate::errors::{AppError, AppResult};
use rusqlite::{params, OptionalExtension, Transaction};
use serde::Serialize;

/// Icons a folder may use (rendered by the shell from its icon set).
pub const FOLDER_ICONS: &[&str] = &[
    "folder",
    "star",
    "heart",
    "briefcase",
    "book",
    "code",
    "music",
    "film",
    "gamepad",
    "cart",
    "news",
    "school",
    "plane",
    "home",
    "coffee",
    "globe",
    "wrench",
    "sparkles",
];

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Bookmark {
    pub id: i64,
    pub parent_id: Option<i64>,
    /// "bookmark" or "folder"
    pub kind: String,
    pub url: Option<String>,
    pub title: String,
    pub icon: Option<String>,
    pub color: Option<String>,
    /// The site's icon as a PNG data URL (bookmarks only).
    pub favicon: Option<String>,
    pub position: i64,
    pub created_at: i64,
}

/// Input tree for imports.
#[derive(Debug, Clone, PartialEq)]
pub enum ImportNode {
    Folder {
        title: String,
        children: Vec<ImportNode>,
    },
    Link {
        title: String,
        url: String,
        /// Icon embedded in an exported file (validated and shrunk before storing).
        icon: Option<String>,
    },
}

const MAX_IMPORT_ITEMS: usize = 50_000;

fn validate_color(color: Option<&str>) -> AppResult<()> {
    match color {
        Some(c) if !crate::browser::tabs::valid_group_color(c) => {
            Err(AppError::InvalidInput("invalid colour".into()))
        }
        _ => Ok(()),
    }
}

fn validate_parent(conn: &rusqlite::Connection, parent: Option<i64>) -> AppResult<()> {
    if let Some(p) = parent {
        let kind: Option<String> = conn
            .query_row("SELECT kind FROM bookmarks WHERE id = ?1", [p], |r| {
                r.get(0)
            })
            .optional()?;
        if kind.as_deref() != Some("folder") {
            return Err(AppError::InvalidInput("parent must be a folder".into()));
        }
    }
    Ok(())
}

fn web_url(url: &str) -> AppResult<String> {
    crate::browser::url::web_url(url.trim())
        .map(String::from)
        .ok_or_else(|| {
            AppError::InvalidInput("enter a web address starting with http:// or https://".into())
        })
}

fn validate_icon(icon: Option<&str>) -> AppResult<()> {
    match icon {
        Some(i) if !FOLDER_ICONS.contains(&i) => {
            Err(AppError::InvalidInput("unknown folder icon".into()))
        }
        _ => Ok(()),
    }
}

/// `https://host[:port]/`: every bookmark URL of a site starts with it.
fn site_prefix(url: &str) -> Option<String> {
    crate::browser::url::origin_of(url).map(|o| format!("{o}/"))
}

/// An icon another bookmark of the same site already has.
fn known_favicon(conn: &rusqlite::Connection, url: &str) -> rusqlite::Result<Option<String>> {
    let Some(prefix) = site_prefix(url) else {
        return Ok(None);
    };
    conn.query_row(
        "SELECT favicon FROM bookmarks WHERE kind = 'bookmark' AND favicon IS NOT NULL
           AND substr(url, 1, length(?1)) = ?1 LIMIT 1",
        [prefix],
        |r| r.get(0),
    )
    .optional()
}

fn next_position(tx: &rusqlite::Connection, parent: Option<i64>) -> rusqlite::Result<i64> {
    tx.query_row(
        "SELECT COALESCE(MAX(position), -1) + 1 FROM bookmarks WHERE parent_id IS ?1",
        [parent],
        |r| r.get(0),
    )
}

const BOOKMARK_COLUMNS: &str =
    "SELECT id, parent_id, kind, url, title, icon, position, created_at, color, favicon FROM bookmarks";

fn bookmark_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Bookmark> {
    Ok(Bookmark {
        id: r.get(0)?,
        parent_id: r.get(1)?,
        kind: r.get(2)?,
        url: r.get(3)?,
        title: r.get(4)?,
        icon: r.get(5)?,
        position: r.get(6)?,
        created_at: r.get(7)?,
        color: r.get(8)?,
        favicon: r.get(9)?,
    })
}

/// Drops links that are not http(s) and everything past MAX_IMPORT_ITEMS rows (folders count too),
/// then shrinks the kept icons. Runs before the DB lock, and before decoding, so icons past the cap
/// are never decoded. `rows` follows insertion order: a folder, then its children.
fn prepare_import(nodes: &mut Vec<ImportNode>, rows: &mut usize, skipped: &mut usize) {
    nodes.retain_mut(|node| {
        if *rows >= MAX_IMPORT_ITEMS {
            *skipped += link_count(node);
            return false;
        }
        match node {
            ImportNode::Link { url, icon, .. } => {
                // Bookmarklets (javascript:), file: and other schemes are dropped on purpose.
                let Some(u) = crate::browser::url::web_url(url) else {
                    *skipped += 1;
                    return false;
                };
                *url = u.into();
                *icon = icon.as_deref().and_then(favicon::normalize);
                *rows += 1;
            }
            ImportNode::Folder { children, .. } => {
                *rows += 1;
                prepare_import(children, rows, skipped);
            }
        }
        true
    });
}

fn link_count(node: &ImportNode) -> usize {
    match node {
        ImportNode::Link { .. } => 1,
        ImportNode::Folder { children, .. } => children.iter().map(link_count).sum(),
    }
}

/// Result of an import, shown to the user.
#[derive(Serialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImportSummary {
    pub imported: usize,
    /// Links left out: unsupported schemes and everything past the import limit.
    pub skipped: usize,
}

impl Db {
    /// Every node, parents before children, in display order.
    pub fn bookmarks_list(&self) -> AppResult<Vec<Bookmark>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "{BOOKMARK_COLUMNS} ORDER BY parent_id IS NOT NULL, parent_id, position, id"
        ))?;
        let rows = stmt.query_map([], bookmark_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn bookmark_get(&self, id: i64) -> AppResult<Bookmark> {
        self.conn()
            .query_row(
                &format!("{BOOKMARK_COLUMNS} WHERE id = ?1"),
                [id],
                bookmark_row,
            )
            .optional()?
            .ok_or_else(|| AppError::NotFound("bookmark".into()))
    }

    pub fn bookmark_is_saved(&self, url: &str) -> AppResult<bool> {
        Ok(self
            .conn()
            .query_row(
                "SELECT 1 FROM bookmarks WHERE url = ?1 LIMIT 1",
                [url],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// Ctrl+D behaviour: removes every bookmark for the URL, or adds one to the bar with the page's
    /// current icon (or one already known for the site). Returns the new saved state.
    pub fn bookmark_toggle(&self, url: &str, title: &str, icon: Option<&str>) -> AppResult<bool> {
        let conn = self.conn();
        if conn.execute(
            "DELETE FROM bookmarks WHERE kind = 'bookmark' AND url = ?1",
            [url],
        )? > 0
        {
            return Ok(false);
        }
        let pos = next_position(&conn, None)?;
        let icon = match icon.and_then(favicon::normalize) {
            Some(i) => Some(i),
            None => known_favicon(&conn, url)?,
        };
        conn.execute(
            "INSERT INTO bookmarks (parent_id, kind, url, title, favicon, position, created_at) VALUES (NULL, 'bookmark', ?1, ?2, ?3, ?4, ?5)",
            params![url, title, icon, pos, now_millis()],
        )?;
        Ok(true)
    }

    pub fn bookmark_create_folder(
        &self,
        parent: Option<i64>,
        title: &str,
        icon: Option<&str>,
        color: Option<&str>,
    ) -> AppResult<i64> {
        validate_icon(icon)?;
        validate_color(color)?;
        let conn = self.conn();
        validate_parent(&conn, parent)?;
        let pos = next_position(&conn, parent)?;
        conn.execute(
            "INSERT INTO bookmarks (parent_id, kind, url, title, icon, color, position, created_at) VALUES (?1, 'folder', NULL, ?2, ?3, ?4, ?5, ?6)",
            params![parent, title, icon, color, pos, now_millis()],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Adds a bookmark with a user-entered address (only http/https).
    pub fn bookmark_add(&self, parent: Option<i64>, title: &str, url: &str) -> AppResult<i64> {
        let url = web_url(url)?;
        let conn = self.conn();
        validate_parent(&conn, parent)?;
        let pos = next_position(&conn, parent)?;
        // An empty title is allowed: the bar then shows only the site icon.
        let title = title.trim();
        let icon = known_favicon(&conn, &url)?;
        conn.execute(
            "INSERT INTO bookmarks (parent_id, kind, url, title, favicon, position, created_at) VALUES (?1, 'bookmark', ?2, ?3, ?4, ?5, ?6)",
            params![parent, url, title, icon, pos, now_millis()],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Renames a node. Folders take `icon`/`color` (None clears); bookmarks take a new `url` if given.
    pub fn bookmark_update(
        &self,
        id: i64,
        title: &str,
        icon: Option<&str>,
        color: Option<&str>,
        url: Option<&str>,
    ) -> AppResult<()> {
        validate_icon(icon)?;
        validate_color(color)?;
        let url = url.map(web_url).transpose()?;
        let conn = self.conn();
        // A new address may be another site: take that site's known icon (or none).
        let new_icon = match &url {
            Some(u) => known_favicon(&conn, u)?,
            None => None,
        };
        let n = conn.execute(
            "UPDATE bookmarks SET title = ?1,
               icon = CASE WHEN kind = 'folder' THEN ?2 ELSE icon END,
               color = CASE WHEN kind = 'folder' THEN ?3 ELSE color END,
               favicon = CASE WHEN kind = 'bookmark' AND ?4 IS NOT NULL AND url IS NOT ?4 THEN ?6 ELSE favicon END,
               url = CASE WHEN kind = 'bookmark' AND ?4 IS NOT NULL THEN ?4 ELSE url END
             WHERE id = ?5",
            params![title, icon, color, url, id, new_icon],
        )?;
        if n == 0 {
            return Err(AppError::NotFound("bookmark".into()));
        }
        Ok(())
    }

    /// Moves a node to the end of `parent` (None = bookmark bar). Folders cannot move into themselves.
    pub fn bookmark_move(&self, id: i64, parent: Option<i64>) -> AppResult<()> {
        let conn = self.conn();
        if let Some(p) = parent {
            let inside_self: bool = conn.query_row(
                "WITH RECURSIVE up(id, parent_id) AS (SELECT id, parent_id FROM bookmarks WHERE id = ?1
                   UNION ALL SELECT b.id, b.parent_id FROM bookmarks b JOIN up ON b.id = up.parent_id)
                 SELECT EXISTS(SELECT 1 FROM up WHERE id = ?2)",
                params![p, id],
                |r| r.get(0),
            )?;
            let is_folder: bool = conn
                .query_row(
                    "SELECT kind = 'folder' FROM bookmarks WHERE id = ?1",
                    [p],
                    |r| r.get(0),
                )
                .optional()?
                .unwrap_or(false);
            if inside_self || !is_folder {
                return Err(AppError::InvalidInput("cannot move there".into()));
            }
        }
        let pos = next_position(&conn, parent)?;
        conn.execute(
            "UPDATE bookmarks SET parent_id = ?1, position = ?2 WHERE id = ?3",
            params![parent, pos, id],
        )?;
        Ok(())
    }

    /// Gives every bookmark of a page's site (same origin) the icon the page just reported, so
    /// bookmarks of pages not visited yet get it too. Sites without bookmarks keep nothing, so this
    /// records no browsing. Returns how many bookmarks changed.
    pub fn bookmark_set_site_favicon(&self, page_url: &str, data_url: &str) -> AppResult<usize> {
        let Some(prefix) = site_prefix(page_url) else {
            return Ok(0);
        };
        let bookmarked = self
            .conn()
            .query_row(
                "SELECT 1 FROM bookmarks WHERE kind = 'bookmark' AND substr(url, 1, length(?1)) = ?1 LIMIT 1",
                [&prefix],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        // Decoded without the lock held: other threads (suggestions, imports) share the connection.
        let Some(icon) = bookmarked.then(|| favicon::normalize(data_url)).flatten() else {
            return Ok(0);
        };
        // `IS NOT ?1` leaves rows that already have this icon untouched.
        Ok(self.conn().execute(
            "UPDATE bookmarks SET favicon = ?1
             WHERE kind = 'bookmark' AND substr(url, 1, length(?2)) = ?2 AND favicon IS NOT ?1",
            params![icon, prefix],
        )?)
    }

    /// Deletes a node and, for folders, everything inside it.
    pub fn bookmark_remove(&self, id: i64) -> AppResult<()> {
        self.conn().execute(
            "WITH RECURSIVE sub(id) AS (SELECT ?1 UNION ALL SELECT b.id FROM bookmarks b JOIN sub ON b.parent_id = sub.id)
             DELETE FROM bookmarks WHERE id IN sub",
            [id],
        )?;
        Ok(())
    }

    /// Imports a tree into a new folder on the bookmark bar. Only http(s) links are kept.
    pub fn bookmarks_import(
        &self,
        folder_title: &str,
        mut nodes: Vec<ImportNode>,
    ) -> AppResult<ImportSummary> {
        let mut summary = ImportSummary::default();
        prepare_import(&mut nodes, &mut 0, &mut summary.skipped);
        if nodes.is_empty() {
            return Ok(summary);
        }
        // shortcut: one transaction for up to 50k rows holds the DB lock, so history writes wait
        // for the whole import; commit in chunks if imports get slow.
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let pos = next_position(&tx, None)?;
        tx.execute(
            "INSERT INTO bookmarks (parent_id, kind, url, title, icon, position, created_at) VALUES (NULL, 'folder', NULL, ?1, 'folder', ?2, ?3)",
            params![folder_title, pos, now_millis()],
        )?;
        let root = tx.last_insert_rowid();
        insert_nodes(&tx, root, &nodes, &mut summary.imported)?;
        tx.commit()?;
        Ok(summary)
    }
}

/// Inserts a tree already checked by `prepare_import`; `count` is bookmarks inserted.
fn insert_nodes(
    tx: &Transaction<'_>,
    parent: i64,
    nodes: &[ImportNode],
    count: &mut usize,
) -> AppResult<()> {
    let now = now_millis();
    for (position, node) in nodes.iter().enumerate() {
        match node {
            ImportNode::Link { title, url, icon } => {
                // Untitled links stay untitled (icon-only on the bar, as in the source browser).
                let title = title.trim();
                tx.prepare_cached(
                    "INSERT INTO bookmarks (parent_id, kind, url, title, favicon, position, created_at) VALUES (?1, 'bookmark', ?2, ?3, ?4, ?5, ?6)",
                )?
                .execute(params![parent, url, truncate(title), icon, position as i64, now])?;
                *count += 1;
            }
            ImportNode::Folder { title, children } => {
                tx.prepare_cached(
                    "INSERT INTO bookmarks (parent_id, kind, url, title, icon, position, created_at) VALUES (?1, 'folder', NULL, ?2, 'folder', ?3, ?4)",
                )?
                .execute(params![parent, truncate(title.trim()), position as i64, now])?;
                let id = tx.last_insert_rowid();
                insert_nodes(tx, id, children, count)?;
            }
        }
    }
    Ok(())
}

fn truncate(s: &str) -> String {
    s.chars().take(1024).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A valid 1×1 PNG.
    const PNG_1X1: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";

    #[test]
    fn toggle_adds_to_bar_then_removes() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.bookmark_toggle("https://a.example/", "A", None).unwrap());
        assert!(db.bookmark_is_saved("https://a.example/").unwrap());
        let b = &db.bookmarks_list().unwrap()[0];
        assert_eq!((b.parent_id, b.kind.as_str()), (None, "bookmark"));
        db.bookmark_update(b.id, "Renamed", None, None, None)
            .unwrap();
        assert_eq!(db.bookmarks_list().unwrap()[0].title, "Renamed");
        assert!(!db.bookmark_toggle("https://a.example/", "A", None).unwrap());
        assert!(db.bookmarks_list().unwrap().is_empty());
        assert!(db.bookmark_update(b.id, "x", None, None, None).is_err());
        assert!(matches!(db.bookmark_get(b.id), Err(AppError::NotFound(_))));
    }

    #[test]
    fn folders_icons_moves_and_cascading_delete() {
        let db = Db::open_in_memory().unwrap();
        let f = db
            .bookmark_create_folder(None, "Work", Some("briefcase"), Some("blue"))
            .unwrap();
        let sub = db
            .bookmark_create_folder(Some(f), "Sub", None, None)
            .unwrap();
        db.bookmark_toggle("https://a.example/", "A", None).unwrap();
        let a = db
            .bookmarks_list()
            .unwrap()
            .into_iter()
            .find(|b| b.kind == "bookmark")
            .unwrap();
        db.bookmark_move(a.id, Some(sub)).unwrap();
        assert_eq!(db.bookmark_get(a.id).unwrap().parent_id, Some(sub));
        assert!(
            db.bookmark_move(f, Some(sub)).is_err(),
            "folder into its own child"
        );
        assert!(
            db.bookmark_move(f, Some(a.id)).is_err(),
            "into a non-folder"
        );
        assert!(db
            .bookmark_create_folder(None, "X", Some("not-an-icon"), None)
            .is_err());
        db.bookmark_update(f, "Work", Some("code"), Some("#12ab9f"), None)
            .unwrap();
        assert_eq!(
            db.bookmark_get(f).unwrap().color.as_deref(),
            Some("#12ab9f")
        );
        assert!(db
            .bookmark_update(f, "Work", None, Some("url(x)"), None)
            .is_err());
        assert_eq!(db.bookmark_get(f).unwrap().icon.as_deref(), Some("code"));
        db.bookmark_remove(f).unwrap();
        assert!(db.bookmarks_list().unwrap().is_empty());
    }

    #[test]
    fn custom_bookmarks_validate_urls_and_can_be_edited() {
        let db = Db::open_in_memory().unwrap();
        let id = db
            .bookmark_add(None, "  ", "https://custom.example/x")
            .unwrap();
        assert_eq!(
            db.bookmark_get(id).unwrap().title,
            "",
            "blank title stays blank"
        );
        assert!(db.bookmark_add(None, "Bad", "javascript:alert(1)").is_err());
        assert!(db
            .bookmark_add(Some(id), "In a bookmark?", "https://a.example/")
            .is_err());
        db.bookmark_update(id, "Custom", None, None, Some("https://new.example/"))
            .unwrap();
        let b = db.bookmark_get(id).unwrap();
        assert_eq!(
            (b.title.as_str(), b.url.as_deref()),
            ("Custom", Some("https://new.example/"))
        );
        assert!(db
            .bookmark_update(id, "Custom", None, None, Some("file:///C:/"))
            .is_err());
    }

    #[test]
    fn import_keeps_structure_and_drops_unsafe_links() {
        let db = Db::open_in_memory().unwrap();
        let nodes = vec![
            ImportNode::Link {
                title: "A".into(),
                url: "https://a.example/".into(),
                icon: Some(PNG_1X1.into()),
            },
            ImportNode::Link {
                title: "bad".into(),
                url: "javascript:alert(1)".into(),
                icon: None,
            },
            ImportNode::Folder {
                title: "News".into(),
                children: vec![ImportNode::Link {
                    title: "".into(),
                    url: "https://n.example/".into(),
                    icon: Some("data:image/svg+xml;base64,PHN2Zy8+".into()),
                }],
            },
        ];
        assert_eq!(
            db.bookmarks_import("Imported", nodes).unwrap(),
            ImportSummary {
                imported: 2,
                skipped: 1
            }
        );
        let all = db.bookmarks_list().unwrap();
        let root = all.iter().find(|b| b.title == "Imported").unwrap();
        assert_eq!(root.parent_id, None);
        let news = all.iter().find(|b| b.title == "News").unwrap();
        assert_eq!(news.parent_id, Some(root.id));
        let n = all
            .iter()
            .find(|b| b.url.as_deref() == Some("https://n.example/"))
            .unwrap();
        assert_eq!((n.parent_id, n.title.as_str()), (Some(news.id), ""));
        assert_eq!(n.favicon, None, "only PNG icons are kept");
        let a = all.iter().find(|b| b.title == "A").unwrap();
        assert_eq!(a.favicon, favicon::normalize(PNG_1X1));
        assert!(a.favicon.is_some());
    }

    #[test]
    fn an_import_with_nothing_to_keep_creates_no_folder() {
        let db = Db::open_in_memory().unwrap();
        let nodes = vec![ImportNode::Link {
            title: "bad".into(),
            url: "javascript:alert(1)".into(),
            icon: None,
        }];
        assert_eq!(
            db.bookmarks_import("Imported", nodes).unwrap(),
            ImportSummary {
                imported: 0,
                skipped: 1
            }
        );
        assert!(db.bookmarks_list().unwrap().is_empty());
    }

    #[test]
    fn import_limit_counts_folders() {
        let db = Db::open_in_memory().unwrap();
        let folder = || ImportNode::Folder {
            title: "F".into(),
            children: Vec::new(),
        };
        let mut nodes = vec![folder(); MAX_IMPORT_ITEMS + 5];
        // Past the cap: skipped, and its (invalid) icon is never decoded.
        nodes.push(ImportNode::Folder {
            title: "Late".into(),
            children: vec![ImportNode::Link {
                title: "L".into(),
                url: "https://late.example/".into(),
                icon: Some("data:image/png;base64,!!".into()),
            }],
        });
        assert_eq!(
            db.bookmarks_import("Imported", nodes).unwrap(),
            ImportSummary {
                imported: 0,
                skipped: 1
            }
        );
        // The import root plus the capped folders.
        assert_eq!(db.bookmarks_list().unwrap().len(), MAX_IMPORT_ITEMS + 1);
    }

    #[test]
    fn site_icons_reach_every_bookmark_of_the_site_and_nothing_else() {
        let db = Db::open_in_memory().unwrap();
        let x = db.bookmark_add(None, "X", "https://a.example/x").unwrap();
        let y = db
            .bookmark_add(None, "Y", "https://a.example/y?q=1")
            .unwrap();
        let other = db.bookmark_add(None, "B", "https://b.example/").unwrap();
        let set = |page: &str| db.bookmark_set_site_favicon(page, PNG_1X1).unwrap();
        assert_eq!(set("https://a.example/elsewhere"), 2);
        assert_eq!(
            set("https://a.example/"),
            0,
            "unchanged icons are not rewritten"
        );
        let stored = favicon::normalize(PNG_1X1).unwrap();
        let icon = |id| db.bookmark_get(id).unwrap().favicon;
        assert_eq!(icon(x).as_deref(), Some(stored.as_str()));
        assert_eq!(icon(y).as_deref(), Some(stored.as_str()));
        assert_eq!(icon(other), None);
        // A site without bookmarks keeps nothing; a lookalike host is another site.
        assert_eq!(set("https://a.example.evil/"), 0);
        // New bookmarks of a known site start with its icon; moving to another site drops it.
        let z = db.bookmark_add(None, "Z", "https://a.example/z").unwrap();
        assert_eq!(icon(z).as_deref(), Some(stored.as_str()));
        db.bookmark_update(z, "Z", None, None, Some("https://c.example/"))
            .unwrap();
        assert_eq!(icon(z), None);
        assert!(db
            .bookmark_toggle("https://a.example/t", "T", None)
            .unwrap());
        let t = db.bookmarks_list().unwrap();
        let t = t.iter().find(|b| b.title == "T").unwrap();
        assert_eq!(t.favicon.as_deref(), Some(stored.as_str()));
    }

    #[test]
    fn migration_from_flat_bookmarks_keeps_rows() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(super::super::MIGRATIONS[0]).unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
        conn.execute("INSERT INTO bookmarks (url, title, created_at) VALUES ('https://old.example/', 'Old', 1)", []).unwrap();
        super::super::migrate(&mut conn).unwrap();
        let kind: String = conn
            .query_row(
                "SELECT kind FROM bookmarks WHERE url = 'https://old.example/'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(kind, "bookmark");
    }
}
