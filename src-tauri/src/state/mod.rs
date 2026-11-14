//! Thread-safe services shared by commands, the browser core and background tasks.
//! Browser/tab state is *not* here: it lives on the UI thread in `browser::with_browser`.

use crate::adblock::AdBlockEngine;
use crate::config::Settings;
use crate::errors::{AppError, AppResult};
use crate::storage::Db;
use std::sync::{Arc, RwLock};

pub struct Services {
    pub db: Arc<Db>,
    pub adblock: Arc<AdBlockEngine>,
    /// Shared with the platform engine (HTTPS-only checks happen inside request handlers).
    pub settings_cell: Arc<RwLock<Settings>>,
}

impl Services {
    pub fn settings(&self) -> Settings {
        self.settings_cell
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Validates, persists and publishes new settings. Returns the previous value.
    pub fn replace_settings(&self, next: Settings) -> AppResult<Settings> {
        next.validate().map_err(AppError::InvalidInput)?;
        self.db.settings_save(&next)?;
        self.adblock.set_enabled(next.adblock_enabled);
        self.adblock.set_cosmetic(next.cosmetic_filtering);
        let mut guard = self
            .settings_cell
            .write()
            .unwrap_or_else(|p| p.into_inner());
        Ok(std::mem::replace(&mut *guard, next))
    }
}
