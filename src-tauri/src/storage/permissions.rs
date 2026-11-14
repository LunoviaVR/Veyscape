use super::Db;
use crate::errors::AppResult;
use rusqlite::{params, OptionalExtension};
use serde::Serialize;

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SitePermission {
    pub origin: String,
    pub kind: String,
    pub state: String,
}

impl Db {
    pub fn permission_get(&self, origin: &str, kind: &str) -> AppResult<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT state FROM site_permissions WHERE origin = ?1 AND kind = ?2",
                params![origin, kind],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// `state = None` removes the stored decision (the site will be asked again).
    pub fn permission_set(&self, origin: &str, kind: &str, state: Option<&str>) -> AppResult<()> {
        let conn = self.conn();
        match state {
            Some(s) => conn.execute(
                "INSERT INTO site_permissions (origin, kind, state) VALUES (?1, ?2, ?3)
                 ON CONFLICT(origin, kind) DO UPDATE SET state = excluded.state",
                params![origin, kind, s],
            )?,
            None => conn.execute(
                "DELETE FROM site_permissions WHERE origin = ?1 AND kind = ?2",
                params![origin, kind],
            )?,
        };
        Ok(())
    }

    pub fn permissions_list(&self) -> AppResult<Vec<SitePermission>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare("SELECT origin, kind, state FROM site_permissions ORDER BY origin, kind")?;
        let rows = stmt.query_map([], |r| {
            Ok(SitePermission {
                origin: r.get(0)?,
                kind: r.get(1)?,
                state: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn permissions_clear(&self) -> AppResult<()> {
        self.conn().execute("DELETE FROM site_permissions", [])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_revoke() {
        let db = Db::open_in_memory().unwrap();
        db.permission_set("https://a.example", "camera", Some("allow"))
            .unwrap();
        db.permission_set("https://a.example", "camera", Some("deny"))
            .unwrap();
        assert_eq!(
            db.permission_get("https://a.example", "camera")
                .unwrap()
                .as_deref(),
            Some("deny")
        );
        assert_eq!(db.permissions_list().unwrap().len(), 1);
        db.permission_set("https://a.example", "camera", None)
            .unwrap();
        assert_eq!(
            db.permission_get("https://a.example", "camera").unwrap(),
            None
        );
    }
}
