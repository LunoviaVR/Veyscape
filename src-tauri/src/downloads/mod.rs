//! Download manager state. The engine performs the transfer (no second network stack);
//! this module tracks items, persists non-private ones, and answers "is this file risky to open".

use crate::platform::DownloadState;
use crate::storage::{now_millis, Db, DownloadRecord};
use serde::Serialize;
use std::sync::atomic::{AtomicI64, Ordering};

static NEXT_ID: AtomicI64 = AtomicI64::new(1);

/// Seeds ids above any persisted row so new downloads never collide with history.
pub fn init_ids(db: &Db) {
    let max = db.downloads_max_id().unwrap_or(0);
    NEXT_ID.store(max + 1, Ordering::Relaxed);
}

pub fn next_id() -> i64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DownloadItem {
    pub id: i64,
    pub url: String,
    pub path: String,
    pub file_name: String,
    pub state: String,
    pub received: i64,
    pub total: Option<i64>,
    pub started_at: i64,
    pub private: bool,
    /// Executables and scripts: the UI asks before opening them.
    pub risky: bool,
}

impl DownloadItem {
    pub fn new(id: i64, url: String, path: String, total: Option<i64>, private: bool) -> Self {
        let mut item = DownloadItem {
            id,
            url,
            path: String::new(),
            file_name: String::new(),
            state: DownloadState::InProgress.as_str().into(),
            received: 0,
            total,
            started_at: now_millis(),
            private,
            risky: false,
        };
        item.set_path(path);
        item
    }

    pub fn from_record(r: DownloadRecord) -> Self {
        let mut item = DownloadItem {
            id: r.id,
            url: r.url,
            path: String::new(),
            file_name: String::new(),
            state: r.state,
            received: r.received,
            total: r.total,
            started_at: r.started_at,
            private: false,
            risky: false,
        };
        item.set_path(r.path);
        item
    }

    pub fn set_path(&mut self, path: String) {
        self.file_name = std::path::Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.risky = is_risky_file(&self.file_name);
        self.path = path;
    }

    pub fn record(&self) -> DownloadRecord {
        DownloadRecord {
            id: self.id,
            url: self.url.clone(),
            path: self.path.clone(),
            state: self.state.clone(),
            received: self.received,
            total: self.total,
            started_at: self.started_at,
        }
    }
}

/// File types that execute code when opened on Windows.
pub fn is_risky_file(name: &str) -> bool {
    const RISKY: &[&str] = &[
        "exe",
        "msi",
        "msix",
        "appx",
        "msixbundle",
        "appxbundle",
        "appinstaller",
        "msp",
        "mst",
        "diagcab",
        "gadget",
        "bat",
        "cmd",
        "com",
        "scr",
        "pif",
        "ps1",
        "psm1",
        "vb",
        "vbs",
        "vbe",
        "js",
        "jse",
        "ws",
        "wsf",
        "wsh",
        "wsc",
        "sct",
        "scf",
        "ps1xml",
        "psc1",
        "hta",
        "jar",
        "lnk",
        "reg",
        "dll",
        "cpl",
        "msc",
        "url",
        "iso",
        "img",
        "vhd",
        "vhdx",
        "application",
        "appref-ms",
        "library-ms",
        "search-ms",
        "settingcontent-ms",
        "chm",
        "xll",
        "inf",
    ];
    let lower = name.to_ascii_lowercase();
    let ext = lower
        .trim_end_matches(['.', ' '])
        .rsplit('.')
        .next()
        .unwrap_or_default();
    lower.contains('.') && RISKY.contains(&ext)
}

/// Drops finished private downloads from the in-memory list (they were never saved). Returns
/// whether anything was removed. Running ones go when they finish.
pub fn forget_private(items: &mut Vec<DownloadItem>) -> bool {
    let before = items.len();
    items.retain(|d| !d.private || d.state == "inProgress");
    items.len() != before
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn risky_extensions() {
        assert!(is_risky_file("setup.EXE"));
        assert!(is_risky_file("invoice.pdf.exe"));
        assert!(is_risky_file("trick.exe. "));
        assert!(is_risky_file("x.ps1"));
        for name in [
            "a.appinstaller",
            "a.msixbundle",
            "a.msp",
            "a.diagcab",
            "a.SCF",
            "a.ps1xml",
        ] {
            assert!(is_risky_file(name), "{name}");
        }
        assert!(!is_risky_file("report.pdf"));
        assert!(!is_risky_file("exe"));
    }

    #[test]
    fn finished_private_downloads_are_forgotten_running_ones_stay() {
        let item = |id, private, state: DownloadState| {
            let mut d =
                DownloadItem::new(id, "https://a.example/f".into(), "f".into(), None, private);
            d.state = state.as_str().into();
            d
        };
        let mut list = vec![
            item(1, true, DownloadState::Completed),
            item(2, true, DownloadState::InProgress),
            item(3, false, DownloadState::Completed),
        ];
        assert!(forget_private(&mut list));
        assert_eq!(list.iter().map(|d| d.id).collect::<Vec<_>>(), [2, 3]);
        assert!(!forget_private(&mut list[1..].to_vec()));
    }

    #[test]
    fn item_derives_file_name() {
        let d = DownloadItem::new(
            1,
            "https://a.example/f".into(),
            "C:\\Users\\me\\Downloads\\f.zip".into(),
            None,
            false,
        );
        assert_eq!(d.file_name, "f.zip");
        assert!(!d.risky);
        assert_eq!(d.state, "inProgress");
    }
}
