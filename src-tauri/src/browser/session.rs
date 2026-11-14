//! Session persistence for crash/restart recovery: every open window with its tabs, groups, split
//! view and geometry. Private tabs are never written.

use super::split::{Split, SplitLayout, MAX_PANES};
use super::tabs::TabManager;
use crate::errors::{AppError, AppResult};
use crate::storage::Db;
use serde::{Deserialize, Serialize};

const KV_SESSION: &str = "session";

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct SavedTab {
    pub url: String,
    pub title: String,
    pub pinned: bool,
    /// Id of the tab's group within `WindowSession::groups`.
    #[serde(default)]
    pub group: Option<u64>,
}

/// Where a window was, in physical pixels: outer position and inner size. Not saved while the
/// window is maximized, minimized or full screen, so those come back at a normal size.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone, Copy)]
pub struct WindowBounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl WindowBounds {
    /// Enough of the window lies on one of the monitors (`(x, y, width, height)`) to grab it, so a
    /// window saved on a monitor that is gone does not reopen off screen.
    pub fn visible_on(&self, monitors: &[(i32, i32, u32, u32)]) -> bool {
        let overlap = |a: i32, a_len: u32, b: i32, b_len: u32| {
            let end = (i64::from(a) + i64::from(a_len)).min(i64::from(b) + i64::from(b_len));
            end - i64::from(a.max(b))
        };
        monitors.iter().any(|&(x, y, w, h)| {
            overlap(self.x, self.width, x, w) >= 100 && overlap(self.y, self.height, y, h) >= 50
        })
    }
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct SavedSplit {
    /// Indices into `WindowSession::tabs`, in pane order.
    pub tabs: Vec<usize>,
    pub layout: SplitLayout,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct WindowSession {
    pub tabs: Vec<SavedTab>,
    /// Index into `tabs` of the active tab.
    pub active: usize,
    #[serde(default)]
    pub groups: Vec<super::tabs::TabGroup>,
    #[serde(default)]
    pub bounds: Option<WindowBounds>,
    #[serde(default)]
    pub maximized: bool,
    #[serde(default)]
    pub split: Option<SavedSplit>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct Session {
    pub windows: Vec<WindowSession>,
}

/// What a window contributes besides its tabs.
#[derive(Default)]
pub struct WindowExtras<'a> {
    pub bounds: Option<WindowBounds>,
    pub maximized: bool,
    pub split: Option<&'a Split>,
}

pub fn capture(tabs: &TabManager, extras: &WindowExtras) -> WindowSession {
    let kept: Vec<_> = tabs.tabs().iter().filter(|t| !t.private).collect();
    let index_of = |id| kept.iter().position(|t| t.id == id);
    let active = tabs.active_id().and_then(index_of).unwrap_or(0);
    // A split with a private pane is left out entirely.
    let split = extras.split.and_then(|s| {
        let panes: Option<Vec<usize>> = s.tabs.iter().map(|id| index_of(*id)).collect();
        panes.filter(|p| p.len() >= 2).map(|tabs| SavedSplit {
            tabs,
            layout: s.layout,
        })
    });
    WindowSession {
        tabs: kept
            .iter()
            .map(|t| SavedTab {
                url: t.url.clone(),
                title: t.title.clone(),
                pinned: t.pinned,
                group: t.group_id,
            })
            .collect(),
        active,
        groups: tabs
            .groups()
            .iter()
            .filter(|g| kept.iter().any(|t| t.group_id == Some(g.id)))
            .cloned()
            .collect(),
        bounds: extras.bounds,
        maximized: extras.maximized,
        split,
    }
}

/// Saves the given windows in order; windows with only private tabs are left out.
pub fn save(db: &Db, windows: impl IntoIterator<Item = WindowSession>) -> AppResult<()> {
    let session = Session {
        windows: windows.into_iter().filter(|w| !w.tabs.is_empty()).collect(),
    };
    let json =
        serde_json::to_string(&session).map_err(|e| AppError::InvalidInput(e.to_string()))?;
    db.kv_set(KV_SESSION, &json)
}

pub fn load(db: &Db) -> Option<Session> {
    let raw = db.kv_get(KV_SESSION).ok()??;
    // Sessions saved before multi-window support hold one window's fields at the top level.
    let session = serde_json::from_str::<Session>(&raw)
        .or_else(|_| {
            serde_json::from_str::<WindowSession>(&raw).map(|w| Session { windows: vec![w] })
        })
        .ok()?;
    let windows = session
        .windows
        .into_iter()
        .map(sanitize)
        .filter(|w| !w.tabs.is_empty())
        .collect();
    Some(Session { windows })
}

/// Keeps only URLs the browser would let a user open, and remaps indices to the tabs that remain.
fn sanitize(w: WindowSession) -> WindowSession {
    let mut new_index = Vec::with_capacity(w.tabs.len());
    let mut tabs = Vec::new();
    for t in w.tabs {
        let allowed =
            super::url::web_url(&t.url).is_some() || super::url::internal_page(&t.url).is_some();
        new_index.push(allowed.then_some(tabs.len()));
        if allowed {
            tabs.push(t);
        }
    }
    let remap = |i: usize| new_index.get(i).copied().flatten();
    let split = w.split.and_then(|s| {
        let mut panes = Vec::with_capacity(s.tabs.len());
        for &i in &s.tabs {
            let pane = remap(i)?;
            if !panes.contains(&pane) {
                panes.push(pane);
            }
        }
        (2..=MAX_PANES)
            .contains(&panes.len())
            .then_some(SavedSplit {
                tabs: panes,
                layout: s.layout,
            })
    });
    let mut groups = w.groups;
    groups.retain(|g| tabs.iter().any(|t| t.group == Some(g.id)));
    WindowSession {
        active: remap(w.active).unwrap_or(0),
        tabs,
        split,
        groups,
        ..w
    }
}

/// Whether saving the captured `windows` must be skipped. `keep` is set when the last window with
/// tabs to save closed while private windows stay open; that saved session is kept until a window
/// again has a tab to save. Otherwise an empty session is saved, so a closed tab stays closed.
pub fn skip_save(windows: &[WindowSession], keep: &mut bool) -> bool {
    if windows.iter().any(|w| !w.tabs.is_empty()) {
        *keep = false;
    }
    *keep
}

/// `saves[i]`: window `i` holds a tab the session would save. True when `closing` is the only
/// one, so the session must be saved with it before it goes.
pub fn is_last_saved_window(closing: usize, saves: &[bool]) -> bool {
    saves.get(closing) == Some(&true) && saves.iter().enumerate().all(|(i, &s)| i == closing || !s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::tabs::Tab;

    fn window(urls: &[(&str, bool)]) -> TabManager {
        let mut m = TabManager::default();
        for (url, private) in urls {
            let id = m.allocate_id();
            m.insert(Tab::new(id, url, *private), None);
        }
        m
    }

    fn plain(m: &TabManager) -> WindowSession {
        capture(m, &WindowExtras::default())
    }

    #[test]
    fn private_tabs_are_not_persisted_and_active_index_is_remapped() {
        let db = Db::open_in_memory().unwrap();
        let mut m = window(&[
            ("https://a.example/", false),
            ("https://p.example/", true),
            ("https://b.example/", false),
        ]);
        m.activate(3);
        save(&db, [plain(&m)]).unwrap();
        let s = load(&db).unwrap();
        assert_eq!(s.windows.len(), 1);
        let w = &s.windows[0];
        assert_eq!(w.tabs.len(), 2);
        assert!(w.tabs.iter().all(|t| !t.url.contains("p.example")));
        assert_eq!(w.active, 1);
    }

    #[test]
    fn every_window_is_saved_except_private_only_ones() {
        let db = Db::open_in_memory().unwrap();
        let a = window(&[("https://a.example/", false)]);
        let p = window(&[("https://p.example/", true)]);
        let b = window(&[("https://b.example/", false), ("https://c.example/", false)]);
        save(&db, [plain(&a), plain(&p), plain(&b)]).unwrap();
        let s = load(&db).unwrap();
        let counts: Vec<usize> = s.windows.iter().map(|w| w.tabs.len()).collect();
        assert_eq!(counts, [1, 2]);
    }

    #[test]
    fn geometry_and_split_round_trip_but_not_a_split_with_a_private_pane() {
        let db = Db::open_in_memory().unwrap();
        let m = window(&[
            ("https://a.example/", false),
            ("https://b.example/", false),
            ("https://p.example/", true),
        ]);
        let bounds = WindowBounds {
            x: 40,
            y: 30,
            width: 1200,
            height: 800,
        };
        let split = Split {
            tabs: vec![2, 1],
            layout: SplitLayout::Rows,
        };
        let with_private = Split {
            tabs: vec![1, 3],
            layout: SplitLayout::Columns,
        };
        let extras = |s| WindowExtras {
            bounds: Some(bounds),
            maximized: true,
            split: Some(s),
        };
        save(
            &db,
            [
                capture(&m, &extras(&split)),
                capture(&m, &extras(&with_private)),
            ],
        )
        .unwrap();
        let s = load(&db).unwrap();
        let w = &s.windows[0];
        assert_eq!((w.bounds, w.maximized), (Some(bounds), true));
        assert_eq!(
            w.split,
            Some(SavedSplit {
                tabs: vec![1, 0],
                layout: SplitLayout::Rows
            })
        );
        assert_eq!(s.windows[1].split, None);
    }

    #[test]
    fn dropped_urls_shift_split_and_active_indices() {
        let db = Db::open_in_memory().unwrap();
        db.kv_set(KV_SESSION, r#"{"windows":[{"tabs":[
            {"url":"file:///C:/x","title":"","pinned":false},
            {"url":"https://a.example/","title":"","pinned":false},
            {"url":"https://b.example/","title":"","pinned":false}],
            "active":2,"split":{"tabs":[1,2],"layout":"columns"}},
            {"tabs":[{"url":"https://c.example/","title":"","pinned":false},{"url":"javascript:1","title":"","pinned":false}],
            "active":0,"split":{"tabs":[0,1],"layout":"rows"}}]}"#)
            .unwrap();
        let s = load(&db).unwrap();
        assert_eq!(s.windows[0].active, 1);
        assert_eq!(
            s.windows[0].split.as_ref().map(|x| x.tabs.clone()),
            Some(vec![0, 1])
        );
        assert_eq!(
            s.windows[1].split, None,
            "a split that lost a pane is dropped"
        );
    }

    #[test]
    fn restored_splits_and_groups_are_sanitized() {
        let db = Db::open_in_memory().unwrap();
        let tab = |url: &str, group: Option<u64>| SavedTab {
            url: url.into(),
            title: String::new(),
            pinned: false,
            group,
        };
        let group = |id| super::super::tabs::TabGroup {
            id,
            title: String::new(),
            color: "blue".into(),
            collapsed: false,
            icon: None,
        };
        let w = |split: Vec<usize>| WindowSession {
            tabs: vec![
                tab("https://a.example/", Some(1)),
                tab("https://b.example/", None),
                tab("file:///C:/x", Some(2)),
                tab("https://c.example/", None),
                tab("https://d.example/", None),
                tab("https://e.example/", None),
            ],
            active: 0,
            groups: vec![group(1), group(2), group(3)],
            bounds: None,
            maximized: false,
            split: Some(SavedSplit {
                tabs: split,
                layout: SplitLayout::Columns,
            }),
        };
        save(
            &db,
            [w(vec![0, 1, 0, 1]), w(vec![0, 0]), w(vec![0, 1, 3, 4, 5])],
        )
        .unwrap();
        let s = load(&db).unwrap();
        let panes = |i: usize| s.windows[i].split.as_ref().map(|x| x.tabs.clone());
        assert_eq!(panes(0), Some(vec![0, 1]), "duplicate panes are merged");
        assert_eq!(panes(1), None, "one distinct pane is no split");
        assert_eq!(panes(2), None, "more than four panes is no split");
        let ids: Vec<u64> = s.windows[0].groups.iter().map(|g| g.id).collect();
        assert_eq!(ids, [1], "groups no remaining tab uses are dropped");
    }

    #[test]
    fn a_session_of_only_private_tabs_is_not_saved_over_the_last_one() {
        let p = window(&[("https://p.example/", true)]);
        let a = window(&[("https://a.example/", false)]);
        let mut keep = true;
        assert!(
            skip_save(&[plain(&p)], &mut keep),
            "last saved window closed"
        );
        assert!(
            skip_save(&[plain(&p)], &mut keep),
            "quitting from the private window still keeps it"
        );
        assert!(!skip_save(&[plain(&p), plain(&a)], &mut keep));
        assert!(!keep, "a tab to save again ends keeping the old session");
        assert!(is_last_saved_window(0, &[true, false, false]));
        assert!(!is_last_saved_window(0, &[true, true]));
        assert!(
            !is_last_saved_window(1, &[true, false]),
            "closing a window with nothing to save"
        );
    }

    #[test]
    fn a_closed_tab_does_not_return_when_only_a_private_tab_is_left_in_a_normal_window() {
        let db = Db::open_in_memory().unwrap();
        save(&db, [plain(&window(&[("https://a.example/", false)]))]).unwrap();
        // Ctrl+Shift+N in the normal window, then its normal tab closed, then quit.
        let p = [plain(&window(&[("https://p.example/", true)]))];
        let mut keep = false;
        assert!(!skip_save(&p, &mut keep));
        save(&db, p).unwrap();
        assert!(load(&db).unwrap().windows.is_empty());
    }

    #[test]
    fn windows_saved_on_a_missing_monitor_are_not_placed() {
        let primary = [(0, 0, 2560, 1440)];
        let at = |x, y| WindowBounds {
            x,
            y,
            width: 1280,
            height: 820,
        };
        assert!(at(100, 100).visible_on(&primary));
        assert!(
            at(2400, 100).visible_on(&primary),
            "partly off screen is fine"
        );
        assert!(
            !at(2500, 100).visible_on(&primary),
            "a sliver is not enough"
        );
        assert!(
            !at(3000, 200).visible_on(&primary),
            "the second monitor is gone"
        );
        assert!(at(3000, 200).visible_on(&[(0, 0, 2560, 1440), (2560, 0, 1920, 1080)]));
    }

    #[test]
    fn single_window_sessions_from_older_versions_still_load() {
        let db = Db::open_in_memory().unwrap();
        db.kv_set(KV_SESSION, r#"{"tabs":[{"url":"file:///C:/x","title":"","pinned":false},{"url":"veyscape://history","title":"","pinned":true}],"active":0}"#)
            .unwrap();
        let s = load(&db).unwrap();
        assert_eq!(s.windows.len(), 1);
        assert_eq!(s.windows[0].tabs.len(), 1, "unsafe URLs are dropped");
        assert_eq!(s.windows[0].tabs[0].url, "veyscape://history");
    }
}
