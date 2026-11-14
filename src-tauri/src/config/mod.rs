//! User settings (persisted as JSON in the `kv` table) and well-known paths.

use serde::{Deserialize, Deserializer, Serialize};
use std::path::PathBuf;
use tauri::Manager;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum StartupBehavior {
    NewTab,
    #[default]
    RestoreSession,
    Homepage,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum SearchEngine {
    #[default]
    DuckDuckGo,
    Brave,
    Startpage,
    Google,
    Bing,
}

impl SearchEngine {
    /// Query URL template; `{}` is replaced by the percent-encoded query.
    pub fn template(self) -> &'static str {
        match self {
            SearchEngine::DuckDuckGo => "https://duckduckgo.com/?q={}",
            SearchEngine::Brave => "https://search.brave.com/search?q={}",
            SearchEngine::Startpage => "https://www.startpage.com/do/search?q={}",
            SearchEngine::Google => "https://www.google.com/search?q={}",
            SearchEngine::Bing => "https://www.bing.com/search?q={}",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum Density {
    #[default]
    Comfortable,
    Compact,
}

/// Maps to WebView2's built-in tracking prevention levels.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum TrackingPrevention {
    Off,
    Basic,
    #[default]
    Balanced,
    Strict,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum NewTabBackground {
    #[default]
    None,
    Mist,
    Dusk,
    Forest,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    #[serde(deserialize_with = "lenient")]
    pub startup: StartupBehavior,
    pub homepage: String,
    #[serde(deserialize_with = "lenient")]
    pub search_engine: SearchEngine,
    /// `None` = the OS Downloads folder.
    pub download_dir: Option<String>,
    #[serde(deserialize_with = "lenient")]
    pub theme: Theme,
    #[serde(deserialize_with = "lenient")]
    pub density: Density,
    pub newtab_show_top_sites: bool,
    pub newtab_show_bookmarks: bool,
    #[serde(deserialize_with = "lenient")]
    pub newtab_background: NewTabBackground,
    pub adblock_enabled: bool,
    pub cosmetic_filtering: bool,
    #[serde(deserialize_with = "lenient")]
    pub tracking_prevention: TrackingPrevention,
    pub https_only: bool,
    pub clear_cookies_on_exit: bool,
    pub show_home_button: bool,
    pub show_bookmark_bar: bool,
    pub offer_to_save_passwords: bool,
}

/// An unknown enum value (from another version) falls back to that field's default instead of
/// failing the whole object, which would reset every setting.
fn lenient<'de, D: Deserializer<'de>, T: Deserialize<'de> + Default>(d: D) -> Result<T, D::Error> {
    Ok(T::deserialize(serde_json::Value::deserialize(d)?).unwrap_or_default())
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            startup: StartupBehavior::RestoreSession,
            homepage: "https://duckduckgo.com/".into(),
            search_engine: SearchEngine::DuckDuckGo,
            download_dir: None,
            theme: Theme::System,
            density: Density::Comfortable,
            newtab_show_top_sites: true,
            newtab_show_bookmarks: false,
            newtab_background: NewTabBackground::None,
            adblock_enabled: true,
            cosmetic_filtering: true,
            tracking_prevention: TrackingPrevention::Balanced,
            https_only: false,
            clear_cookies_on_exit: false,
            show_home_button: true,
            show_bookmark_bar: true,
            offer_to_save_passwords: true,
        }
    }
}

impl Settings {
    /// Rejects values that would be unsafe to act on (e.g. a homepage with a dangerous scheme).
    pub fn validate(&self) -> Result<(), String> {
        if !self.homepage.is_empty() && crate::browser::url::web_url(&self.homepage).is_none() {
            return Err("homepage must be an http(s) URL".into());
        }
        if let Some(dir) = &self.download_dir {
            if !std::path::Path::new(dir).is_absolute() {
                return Err("download folder must be an absolute path".into());
            }
        }
        Ok(())
    }
}

/// Filesystem locations. All browser data stays under the per-user app data folders.
#[derive(Clone, Debug)]
pub struct Paths {
    pub database: PathBuf,
    pub filter_cache: PathBuf,
}

impl Paths {
    pub fn resolve(app: &tauri::AppHandle) -> crate::errors::AppResult<Self> {
        let data = app.path().app_data_dir()?;
        let local = app.path().app_local_data_dir()?;
        std::fs::create_dir_all(&data)?;
        std::fs::create_dir_all(local.join("filters"))?;
        Ok(Paths {
            database: data.join("veyscape.sqlite3"),
            filter_cache: local.join("filters"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_privacy_first() {
        let s = Settings::default();
        assert!(s.adblock_enabled);
        assert_eq!(s.search_engine, SearchEngine::DuckDuckGo);
        assert_ne!(s.tracking_prevention, TrackingPrevention::Off);
        assert!(s.validate().is_ok());
    }

    #[test]
    fn unknown_and_missing_fields_fall_back_to_defaults() {
        let s: Settings = serde_json::from_str(r#"{"theme":"dark","futureField":1}"#).unwrap();
        assert_eq!(s.theme, Theme::Dark);
        assert!(s.adblock_enabled);
        let s: Settings =
            serde_json::from_str(r#"{"searchEngine":"kagi","homepage":"https://x.example"}"#)
                .unwrap();
        assert_eq!(s.search_engine, SearchEngine::DuckDuckGo);
        assert_eq!(s.homepage, "https://x.example");
    }

    #[test]
    fn rejects_dangerous_homepage_and_relative_download_dir() {
        let s = Settings {
            homepage: "javascript:alert(1)".into(),
            ..Default::default()
        };
        assert!(s.validate().is_err());
        let s = Settings {
            download_dir: Some("relative/dir".into()),
            ..Default::default()
        };
        assert!(s.validate().is_err());
    }
}
