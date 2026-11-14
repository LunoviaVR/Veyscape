use super::Db;
use crate::config::Settings;
use crate::errors::{AppError, AppResult};
use rusqlite::{params, OptionalExtension};

impl Db {
    pub fn kv_get(&self, key: &str) -> AppResult<Option<String>> {
        Ok(self
            .conn()
            .query_row("SELECT value FROM kv WHERE key = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    pub fn kv_set(&self, key: &str, value: &str) -> AppResult<()> {
        self.conn().execute(
            "INSERT INTO kv (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Corrupt or outdated JSON falls back to defaults rather than blocking startup.
    pub fn settings_load(&self) -> AppResult<Settings> {
        Ok(match self.kv_get("settings")? {
            Some(json) => serde_json::from_str(&json).unwrap_or_else(|e| {
                log::warn!("settings JSON unreadable, using defaults: {e}");
                Settings::default()
            }),
            None => Settings::default(),
        })
    }

    pub fn settings_save(&self, settings: &Settings) -> AppResult<()> {
        let json =
            serde_json::to_string(settings).map_err(|e| AppError::InvalidInput(e.to_string()))?;
        self.kv_set("settings", &json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(db.settings_load().unwrap(), Settings::default());
        let s = Settings {
            https_only: true,
            ..Default::default()
        };
        db.settings_save(&s).unwrap();
        assert_eq!(db.settings_load().unwrap(), s);
    }

    #[test]
    fn corrupt_settings_fall_back_to_defaults() {
        let db = Db::open_in_memory().unwrap();
        db.kv_set("settings", "{not json").unwrap();
        assert_eq!(db.settings_load().unwrap(), Settings::default());
    }
}
