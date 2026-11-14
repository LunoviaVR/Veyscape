use super::{now_millis, Db};
use crate::errors::{AppError, AppResult};
use rusqlite::{params, OptionalExtension};
use serde::Serialize;

/// A saved login as the UI sees it. The password is never part of it.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Login {
    pub id: i64,
    pub origin: String,
    pub username: String,
    pub created_at: i64,
    pub last_used_at: i64,
}

fn login(r: &rusqlite::Row<'_>) -> rusqlite::Result<Login> {
    Ok(Login {
        id: r.get(0)?,
        origin: r.get(1)?,
        username: r.get(2)?,
        created_at: r.get(3)?,
        last_used_at: r.get(4)?,
    })
}

const COLUMNS: &str = "id, origin, username, created_at, last_used_at";

/// Passwords are stored encrypted (`crate::passwords::protect`); this module only sees blobs.
impl Db {
    pub fn logins_list(&self) -> AppResult<Vec<Login>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM logins ORDER BY origin, username"
        ))?;
        let rows = stmt.query_map([], login)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Most recently used first.
    pub fn logins_for_origin(&self, origin: &str) -> AppResult<Vec<Login>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM logins WHERE origin = ?1 ORDER BY last_used_at DESC, username"
        ))?;
        let rows = stmt.query_map([origin], login)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// (origin, username, encrypted password) of one login.
    pub fn login_secret(&self, id: i64) -> AppResult<Option<(String, String, Vec<u8>)>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT origin, username, password FROM logins WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?)
    }

    /// (id, username, encrypted password) of every login for `origin`.
    pub fn login_secrets_for_origin(&self, origin: &str) -> AppResult<Vec<(i64, String, Vec<u8>)>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare("SELECT id, username, password FROM logins WHERE origin = ?1")?;
        let rows = stmt.query_map([origin], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Saves a login, replacing the password of the same username on the same site.
    pub fn login_save(&self, origin: &str, username: &str, password: &[u8]) -> AppResult<()> {
        let now = now_millis();
        self.conn().execute(
            "INSERT INTO logins (origin, username, password, created_at, last_used_at)
             VALUES (?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(origin, username) DO UPDATE SET password = excluded.password,
                                                         last_used_at = excluded.last_used_at",
            params![origin, username, password, now],
        )?;
        Ok(())
    }

    pub fn login_update(&self, id: i64, username: &str, password: &[u8]) -> AppResult<()> {
        let conn = self.conn();
        let taken: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM logins o JOIN logins l ON l.origin = o.origin
                            WHERE l.id = ?1 AND o.id != ?1 AND o.username = ?2)",
            params![id, username],
            |r| r.get(0),
        )?;
        if taken {
            return Err(AppError::InvalidInput(
                "this site already has a saved login with that username".into(),
            ));
        }
        let n = conn.execute(
            "UPDATE logins SET username = ?2, password = ?3 WHERE id = ?1",
            params![id, username, password],
        )?;
        if n == 0 {
            return Err(AppError::NotFound("login".into()));
        }
        Ok(())
    }

    pub fn login_used(&self, id: i64) -> AppResult<()> {
        self.conn().execute(
            "UPDATE logins SET last_used_at = ?2 WHERE id = ?1",
            params![id, now_millis()],
        )?;
        Ok(())
    }

    pub fn login_delete(&self, id: i64) -> AppResult<()> {
        self.conn()
            .execute("DELETE FROM logins WHERE id = ?1", [id])?;
        self.checkpoint()
    }

    /// Sites the user chose "Never" for.
    pub fn login_never_list(&self) -> AppResult<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT origin FROM login_never ORDER BY origin")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn login_never_has(&self, origin: &str) -> AppResult<bool> {
        Ok(self.conn().query_row(
            "SELECT EXISTS(SELECT 1 FROM login_never WHERE origin = ?1)",
            [origin],
            |r| r.get(0),
        )?)
    }

    pub fn login_never_set(&self, origin: &str, never: bool) -> AppResult<()> {
        let sql = if never {
            "INSERT OR IGNORE INTO login_never (origin) VALUES (?1)"
        } else {
            "DELETE FROM login_never WHERE origin = ?1"
        };
        self.conn().execute(sql, [origin])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "https://a.example";

    #[test]
    fn save_replaces_same_username() {
        let db = Db::open_in_memory().unwrap();
        db.login_save(A, "ann", b"one").unwrap();
        db.login_save(A, "ann", b"two").unwrap();
        db.login_save(A, "bob", b"three").unwrap();
        db.login_save("https://b.example", "ann", b"four").unwrap();
        let mine = db.logins_for_origin(A).unwrap();
        assert_eq!(mine.len(), 2);
        let ann = mine.iter().find(|l| l.username == "ann").unwrap();
        assert_eq!(db.login_secret(ann.id).unwrap().unwrap().2, b"two");
        assert_eq!(db.logins_list().unwrap().len(), 3);
        assert_eq!(db.login_secrets_for_origin(A).unwrap().len(), 2);
    }

    #[test]
    fn update_rejects_duplicate_username() {
        let db = Db::open_in_memory().unwrap();
        db.login_save(A, "ann", b"one").unwrap();
        db.login_save(A, "bob", b"two").unwrap();
        let bob = db
            .logins_list()
            .unwrap()
            .into_iter()
            .find(|l| l.username == "bob")
            .unwrap();
        assert!(db.login_update(bob.id, "ann", b"x").is_err());
        db.login_update(bob.id, "bobby", b"new").unwrap();
        assert_eq!(db.login_secret(bob.id).unwrap().unwrap().1, "bobby");
        db.login_delete(bob.id).unwrap();
        assert!(db.login_secret(bob.id).unwrap().is_none());
        assert!(db.login_update(bob.id, "x", b"y").is_err());
    }

    #[test]
    fn never_list() {
        let db = Db::open_in_memory().unwrap();
        db.login_never_set(A, true).unwrap();
        db.login_never_set(A, true).unwrap();
        assert!(db.login_never_has(A).unwrap());
        assert_eq!(db.login_never_list().unwrap(), vec![A.to_string()]);
        db.login_never_set(A, false).unwrap();
        assert!(!db.login_never_has(A).unwrap());
    }
}
