//! Tab model and ordering rules. Pure state: no engine calls, fully unit-tested.

use super::url::{internal_page, security_of, InternalPage, Security};
use crate::platform::TabId;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TabError {
    /// HTTPS-only mode could not reach the https:// version.
    #[serde(rename_all = "camelCase")]
    HttpsUnavailable {
        http_url: String,
    },
    /// The engine could not load the page; shown by the shell instead of an engine error page.
    #[serde(rename_all = "camelCase")]
    LoadFailed {
        url: String,
        reason: String,
    },
    Crashed,
    Engine {
        message: String,
    },
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Tab {
    pub id: TabId,
    pub url: String,
    pub title: String,
    pub loading: bool,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub pinned: bool,
    pub private: bool,
    pub zoom: f64,
    /// Requests blocked on the current page.
    pub blocked: u32,
    /// Logins saved for this page's site, counted when the page shows a password field.
    pub saved_logins: u32,
    pub security: Security,
    pub internal: Option<InternalPage>,
    pub error: Option<TabError>,
    pub bookmarked: bool,
    pub group_id: Option<u64>,
    /// Favicons travel in a separate event to keep state updates small.
    #[serde(skip)]
    pub favicon: Option<String>,
    /// An engine view exists (or is being created) for this tab.
    #[serde(skip)]
    pub has_view: bool,
    /// Last web URL shown in the view, so "Back" can leave an internal page.
    #[serde(skip)]
    pub web_url: Option<String>,
    /// The view's document title, kept while an internal page covers it: the engine reports a
    /// title only when it changes, so none arrives when the tab returns to the same title.
    #[serde(skip)]
    pub web_title: String,
}

impl Tab {
    pub fn new(id: TabId, url: &str, private: bool) -> Self {
        let mut tab = Tab {
            id,
            url: String::new(),
            title: String::new(),
            loading: false,
            can_go_back: false,
            can_go_forward: false,
            pinned: false,
            private,
            zoom: 1.0,
            blocked: 0,
            saved_logins: 0,
            security: Security::None,
            internal: None,
            error: None,
            bookmarked: false,
            group_id: None,
            favicon: None,
            has_view: false,
            web_url: None,
            web_title: String::new(),
        };
        tab.set_url(url);
        tab
    }

    pub fn set_url(&mut self, url: &str) {
        let was_internal = self.internal.is_some();
        self.url = url.to_string();
        self.security = security_of(url);
        self.internal = internal_page(url);
        if let Some(page) = self.internal {
            self.title = page_title(page).into();
            self.loading = false;
            self.favicon = None;
        } else {
            self.web_url = Some(url.to_string());
            if was_internal {
                self.title = self.web_title.clone();
            }
        }
    }

    /// True when the content area should show this tab's engine view.
    pub fn shows_web_content(&self) -> bool {
        self.internal.is_none() && self.error.is_none() && self.has_view
    }
}

pub fn page_title(page: InternalPage) -> &'static str {
    match page {
        InternalPage::NewTab => "New Tab",
        InternalPage::Settings => "Settings",
        InternalPage::History => "History",
        InternalPage::Bookmarks => "Bookmarks",
        InternalPage::Downloads => "Downloads",
    }
}

/// What "Reopen closed tab" restores. Private tabs are never recorded.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ClosedTab {
    pub url: String,
    pub title: String,
    pub pinned: bool,
    pub index: usize,
}

const MAX_CLOSED: usize = 25;

/// Preset group colours; any `#rrggbb` value is also accepted (custom colour).
pub const GROUP_COLORS: &[&str] = &[
    "grey", "blue", "red", "yellow", "green", "pink", "purple", "cyan", "orange",
];

pub fn valid_group_color(c: &str) -> bool {
    GROUP_COLORS.contains(&c)
        || (c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|ch| ch.is_ascii_hexdigit()))
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TabGroup {
    pub id: u64,
    pub title: String,
    pub color: String,
    /// Collapsed ("minimized") groups show only their chip in the tab strip.
    pub collapsed: bool,
    /// One of the bookmark folder icons (`storage::FOLDER_ICONS`), shown on the chip.
    #[serde(default)]
    pub icon: Option<String>,
}

#[derive(Default)]
pub struct TabManager {
    tabs: Vec<Tab>,
    active: Option<TabId>,
    closed: Vec<ClosedTab>,
    next_id: TabId,
    groups: Vec<TabGroup>,
    next_group: u64,
}

impl TabManager {
    /// Tab ids stay unique across windows: each window counts up from its own base (window id
    /// in the upper 32 bits), which also keeps them exact as JavaScript numbers.
    pub fn for_window(window: u32) -> Self {
        TabManager {
            next_id: TabId::from(window) << 32,
            ..Default::default()
        }
    }

    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }

    pub fn active_id(&self) -> Option<TabId> {
        self.active
    }

    pub fn active(&self) -> Option<&Tab> {
        self.active.and_then(|id| self.get(id))
    }

    pub fn get(&self, id: TabId) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == id)
    }

    pub fn get_mut(&mut self, id: TabId) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    pub fn index_of(&self, id: TabId) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    pub fn can_reopen(&self) -> bool {
        !self.closed.is_empty()
    }

    fn pinned_count(&self) -> usize {
        self.tabs.iter().filter(|t| t.pinned).count()
    }

    pub fn allocate_id(&mut self) -> TabId {
        self.next_id += 1;
        self.next_id
    }

    /// Inserts a tab after `after` (opened from a page) or at the end, respecting the pinned group.
    pub fn insert(&mut self, mut tab: Tab, after: Option<TabId>) {
        // Tabs opened from a grouped tab join its group (they are inserted right after it).
        if !tab.pinned && tab.group_id.is_none() {
            tab.group_id = after.and_then(|id| self.get(id)).and_then(|t| t.group_id);
        }
        let pinned = self.pinned_count();
        let index = match after.and_then(|id| self.index_of(id)) {
            Some(i) => (i + 1).max(if tab.pinned { 0 } else { pinned }),
            None if tab.pinned => pinned,
            None => self.tabs.len(),
        };
        self.tabs.insert(index.min(self.tabs.len()), tab);
    }

    pub fn activate(&mut self, id: TabId) -> bool {
        let exists = self.get(id).is_some();
        if exists {
            self.active = Some(id);
        }
        exists
    }

    /// Removes a tab; if it was active, activates its right neighbour (or left at the end).
    pub fn remove(&mut self, id: TabId) -> Option<Tab> {
        let index = self.index_of(id)?;
        let tab = self.take(id)?;
        if !tab.private && tab.internal != Some(InternalPage::NewTab) {
            self.closed.push(ClosedTab {
                url: tab.url.clone(),
                title: tab.title.clone(),
                pinned: tab.pinned,
                index,
            });
            if self.closed.len() > MAX_CLOSED {
                self.closed.remove(0);
            }
        }
        Some(tab)
    }

    /// Removes a tab without remembering it for "Reopen closed tab" (it moves to another window).
    pub fn take(&mut self, id: TabId) -> Option<Tab> {
        let index = self.index_of(id)?;
        let tab = self.tabs.remove(index);
        if self.active == Some(id) {
            self.active = self
                .tabs
                .get(index)
                .or_else(|| self.tabs.last())
                .map(|t| t.id);
        }
        self.prune_groups();
        Some(tab)
    }

    pub fn pop_closed(&mut self) -> Option<ClosedTab> {
        self.closed.pop()
    }

    /// Moves a tab to `index`, clamped so pinned tabs stay before unpinned ones.
    pub fn move_to(&mut self, id: TabId, index: usize) {
        let Some(from) = self.index_of(id) else {
            return;
        };
        let tab = self.tabs.remove(from);
        let pinned = self.pinned_count();
        let index = if tab.pinned {
            index.min(pinned)
        } else {
            index.clamp(pinned, self.tabs.len())
        };
        self.tabs.insert(index, tab);
        self.fix_membership(index);
    }

    /// Drag and drop onto a group: the tab joins `group` at `index` when that is beside one of
    /// its tabs, otherwise at the group's end.
    pub fn move_into_group(&mut self, id: TabId, index: usize, group: u64) {
        if self.group(group).is_none() {
            return;
        }
        self.move_to(id, index);
        let Some(at) = self.index_of(id) else {
            return;
        };
        if self.tabs[at].pinned {
            return;
        }
        let member = |i: Option<usize>| {
            i.and_then(|i| self.tabs.get(i))
                .is_some_and(|t| t.group_id == Some(group))
        };
        if member(at.checked_sub(1)) || member(Some(at + 1)) {
            self.tabs[at].group_id = Some(group);
            self.prune_groups();
        } else {
            self.assign(id, Some(group));
        }
    }

    /// Keeps groups contiguous after a move: a tab dropped between two members joins their group;
    /// a tab dragged away from its group leaves it.
    fn fix_membership(&mut self, index: usize) {
        if self.tabs[index].pinned {
            return;
        }
        let left = index
            .checked_sub(1)
            .and_then(|i| self.tabs.get(i))
            .and_then(|t| t.group_id);
        let right = self.tabs.get(index + 1).and_then(|t| t.group_id);
        let own = self.tabs[index].group_id;
        self.tabs[index].group_id = if left.is_some() && left == right {
            left
        } else if own.is_some() && own != left && own != right {
            None
        } else {
            own
        };
        self.prune_groups();
    }

    pub fn groups(&self) -> &[TabGroup] {
        &self.groups
    }

    pub fn group(&self, id: u64) -> Option<&TabGroup> {
        self.groups.iter().find(|g| g.id == id)
    }

    pub fn group_tabs(&self, id: u64) -> Vec<TabId> {
        self.tabs
            .iter()
            .filter(|t| t.group_id == Some(id))
            .map(|t| t.id)
            .collect()
    }

    /// Creates a group containing `tab`. Pinned tabs cannot be grouped.
    pub fn create_group(&mut self, tab: TabId, title: &str, color: Option<&str>) -> Option<u64> {
        if self.get(tab).is_none_or(|t| t.pinned) {
            return None;
        }
        self.next_group += 1;
        let id = self.next_group;
        let color = color
            .filter(|c| valid_group_color(c))
            .map(str::to_string)
            .unwrap_or_else(|| GROUP_COLORS[(id as usize - 1) % GROUP_COLORS.len()].to_string());
        self.groups.push(TabGroup {
            id,
            title: title.to_string(),
            color,
            collapsed: false,
            icon: None,
        });
        self.assign(tab, Some(id));
        Some(id)
    }

    /// Puts a tab into a group (moving it next to the group) or takes it out (`None`).
    pub fn assign(&mut self, tab: TabId, group: Option<u64>) {
        let Some(from) = self.index_of(tab) else {
            return;
        };
        if self.tabs[from].pinned || group.is_some_and(|g| self.group(g).is_none()) {
            return;
        }
        let old = self.tabs[from].group_id;
        let mut t = self.tabs.remove(from);
        t.group_id = group;
        // Join: right after the group's last member. Leave: right after the group it left.
        let anchor = group
            .or(old)
            .and_then(|g| self.tabs.iter().rposition(|x| x.group_id == Some(g)));
        let at = anchor.map(|i| i + 1).unwrap_or(from.min(self.tabs.len()));
        self.tabs.insert(at, t);
        self.prune_groups();
    }

    /// Changes the given fields; `icon` is `Some("")` to remove the icon.
    pub fn update_group(
        &mut self,
        id: u64,
        title: Option<&str>,
        color: Option<&str>,
        collapsed: Option<bool>,
        icon: Option<&str>,
    ) -> bool {
        let Some(g) = self.groups.iter_mut().find(|g| g.id == id) else {
            return false;
        };
        if let Some(t) = title {
            g.title = t.trim().chars().take(60).collect();
        }
        if let Some(c) = color.filter(|c| valid_group_color(c)) {
            g.color = c.to_string();
        }
        if let Some(c) = collapsed {
            g.collapsed = c;
        }
        if let Some(i) = icon {
            g.icon = (!i.is_empty()).then(|| i.to_string());
        }
        true
    }

    pub fn ungroup(&mut self, id: u64) {
        self.tabs
            .iter_mut()
            .filter(|t| t.group_id == Some(id))
            .for_each(|t| t.group_id = None);
        self.prune_groups();
    }

    /// Re-creates a group from a saved session; returns its new id.
    pub fn restore_group(&mut self, group: &TabGroup) -> u64 {
        self.next_group += 1;
        let id = self.next_group;
        self.groups.push(TabGroup {
            id,
            ..group.clone()
        });
        id
    }

    fn prune_groups(&mut self) {
        let tabs = &self.tabs;
        self.groups
            .retain(|g| tabs.iter().any(|t| t.group_id == Some(g.id)));
    }

    pub fn set_pinned(&mut self, id: TabId, pinned: bool) {
        let Some(from) = self.index_of(id) else {
            return;
        };
        let mut tab = self.tabs.remove(from);
        tab.pinned = pinned;
        if pinned {
            tab.group_id = None;
        }
        let at = self.pinned_count();
        self.tabs.insert(at, tab);
        self.prune_groups();
    }

    /// Next/previous tab id, wrapping around.
    pub fn cycle(&self, forward: bool) -> Option<TabId> {
        let i = self.active.and_then(|id| self.index_of(id))?;
        let n = self.tabs.len();
        let j = if forward {
            (i + 1) % n
        } else {
            (i + n - 1) % n
        };
        Some(self.tabs[j].id)
    }

    pub fn nth(&self, index: usize) -> Option<TabId> {
        self.tabs.get(index).map(|t| t.id)
    }

    pub fn last(&self) -> Option<TabId> {
        self.tabs.last().map(|t| t.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tm(n: usize) -> TabManager {
        let mut m = TabManager::default();
        for i in 0..n {
            let id = m.allocate_id();
            m.insert(Tab::new(id, &format!("https://t{i}.example/"), false), None);
        }
        m.activate(1);
        m
    }

    fn ids(m: &TabManager) -> Vec<TabId> {
        m.tabs().iter().map(|t| t.id).collect()
    }

    #[test]
    fn tab_ids_do_not_collide_between_windows() {
        let (mut a, mut b) = (TabManager::for_window(1), TabManager::for_window(2));
        let (x, y) = (a.allocate_id(), b.allocate_id());
        assert_ne!(x, y);
        assert!(y < (1 << 53), "ids must stay exact in JavaScript");
    }

    #[test]
    fn closing_active_tab_selects_right_neighbour_then_left() {
        let mut m = tm(3);
        m.activate(2);
        m.remove(2);
        assert_eq!(m.active_id(), Some(3));
        m.remove(3);
        assert_eq!(m.active_id(), Some(1));
        m.remove(1);
        assert_eq!(m.active_id(), None);
    }

    #[test]
    fn closed_tabs_are_reopenable_except_private_and_new_tab() {
        let mut m = tm(1);
        let p = m.allocate_id();
        m.insert(Tab::new(p, "https://secret.example/", true), None);
        let n = m.allocate_id();
        m.insert(Tab::new(n, super::super::url::NEW_TAB_URL, false), None);
        m.remove(p);
        m.remove(n);
        assert!(!m.can_reopen());
        m.remove(1);
        let c = m.pop_closed().unwrap();
        assert_eq!((c.url.as_str(), c.index), ("https://t0.example/", 0));
    }

    #[test]
    fn tabs_open_next_to_opener_and_pinned_group_is_kept() {
        let mut m = tm(3);
        let id = m.allocate_id();
        m.insert(Tab::new(id, "https://child.example/", false), Some(1));
        assert_eq!(ids(&m), vec![1, 4, 2, 3]);
        m.set_pinned(3, true);
        assert_eq!(ids(&m), vec![3, 1, 4, 2]);
        m.move_to(2, 0);
        assert_eq!(
            ids(&m),
            vec![3, 2, 1, 4],
            "unpinned tab cannot move before pinned ones"
        );
        m.move_to(3, 10);
        assert_eq!(
            ids(&m),
            vec![3, 2, 1, 4],
            "pinned tab stays in pinned group"
        );
        m.set_pinned(3, false);
        assert_eq!(ids(&m), vec![3, 2, 1, 4]);
    }

    #[test]
    fn cycling_and_indexing_wrap() {
        let mut m = tm(3);
        m.activate(3);
        assert_eq!(m.cycle(true), Some(1));
        m.activate(1);
        assert_eq!(m.cycle(false), Some(3));
        assert_eq!(m.nth(1), Some(2));
        assert_eq!(m.last(), Some(3));
    }

    #[test]
    fn dropping_onto_a_group_joins_it() {
        let mut m = tm(4);
        let g = m.create_group(2, "Solo", None).unwrap();
        // A one-tab group: no "between two members" spot exists.
        m.move_into_group(4, 1, g);
        assert_eq!(ids(&m), vec![1, 4, 2, 3]);
        assert_eq!(m.get(4).unwrap().group_id, Some(g));
        // Dropped at the far end, away from the group: joins at the group's end.
        m.move_into_group(1, 4, g);
        assert_eq!(ids(&m), vec![4, 2, 1, 3]);
        assert_eq!(m.group_tabs(g), vec![4, 2, 1]);
        assert!(m.get(3).unwrap().group_id.is_none());
    }

    #[test]
    fn taking_a_tab_does_not_offer_to_reopen_it() {
        let mut m = tm(2);
        let t = m.take(1).unwrap();
        assert_eq!(t.id, 1);
        assert!(m.pop_closed().is_none());
        m.remove(2);
        assert!(m.pop_closed().is_some());
    }

    #[test]
    fn groups_stay_contiguous_and_follow_openers() {
        let mut m = tm(4);
        let g = m.create_group(1, "Work", None).unwrap();
        m.assign(3, Some(g));
        assert_eq!(
            ids(&m),
            vec![1, 3, 2, 4],
            "joining moves the tab next to the group"
        );
        let id = m.allocate_id();
        m.insert(Tab::new(id, "https://child.example/", false), Some(1));
        assert_eq!(
            m.get(id).unwrap().group_id,
            Some(g),
            "opened from a grouped tab"
        );
        m.move_to(4, 1);
        assert_eq!(
            m.get(4).unwrap().group_id,
            Some(g),
            "dropped between members joins"
        );
        m.move_to(4, 5);
        assert_eq!(m.get(4).unwrap().group_id, None, "dragged away leaves");
        assert!(m.update_group(
            g,
            Some(" Renamed "),
            Some("#12ab9F"),
            Some(true),
            Some("star")
        ));
        assert_eq!(m.group(g).unwrap().title, "Renamed");
        assert_eq!(m.group(g).unwrap().icon.as_deref(), Some("star"));
        assert!(
            !m.update_group(g, None, Some("javascript:x"), None, None)
                || m.group(g).unwrap().color == "#12ab9F"
        );
        assert!(m.update_group(g, None, None, None, Some("")));
        assert_eq!(m.group(g).unwrap().icon, None, "an empty icon removes it");
        m.ungroup(g);
        assert!(m.groups().is_empty());
    }

    #[test]
    fn pinned_tabs_cannot_be_grouped_and_empty_groups_vanish() {
        let mut m = tm(2);
        m.set_pinned(1, true);
        assert!(m.create_group(1, "", None).is_none());
        let g = m.create_group(2, "", None).unwrap();
        m.remove(2);
        assert!(m.group(g).is_none());
        assert!(valid_group_color("blue") && valid_group_color("#00ff00"));
        assert!(!valid_group_color("#00ff0") && !valid_group_color("url(x)"));
    }

    #[test]
    fn internal_pages_never_show_web_content() {
        let mut t = Tab::new(1, "https://a.example/", false);
        t.has_view = true;
        assert!(t.shows_web_content());
        t.set_url("veyscape://settings");
        assert!(!t.shows_web_content());
        assert_eq!(t.title, "Settings");
        assert_eq!(t.web_url.as_deref(), Some("https://a.example/"));
    }

    #[test]
    fn leaving_an_internal_page_restores_the_view_title() {
        let mut t = Tab::new(1, "https://a.example/", false);
        t.title = "A".into();
        t.web_title = "A".into();
        t.set_url("veyscape://settings");
        t.set_url("https://a.example/");
        assert_eq!(t.title, "A");
        // A web-to-web navigation keeps the title until the engine reports the new one.
        t.title = "Old".into();
        t.set_url("https://b.example/");
        assert_eq!(t.title, "Old");
    }
}
