//! Per-tab session history.
//!
//! The history is a **tree**, not a linear stack: navigating away from an entry that already
//! has forward entries does not discard them - the new page becomes another child of the
//! current entry, so the old path survives as a sibling branch. Back walks to the parent;
//! forward walks to a child (the most recently visited one by default, or one the embedder
//! picks when there are several). A linear back/forward UI is a strict subset: it simply
//! always takes the default forward child.
//!
//! Like [`ScrollState`](super::scroll::ScrollState) this is deliberately pure - no engine
//! dependencies - so it is unit-tested in isolation. The [`TabWorker`](super::worker) owns one
//! per tab, records committed navigations into it, and drives back/forward from it.
//!
//! Entries live in an append-only arena; ids are stable indices and entries are never
//! removed, so an id handed to the embedder stays valid for the tab's lifetime.

use url::Url;

/// Stable identifier of a history entry within one tab. Only meaningful for the tab that
/// issued it.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct HistoryEntryId(pub usize);

/// One entry in a tab's history tree.
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub url: Url,
    /// Document title, updated as the page's title becomes known.
    pub title: Option<String>,
    /// Scroll offset (CSS px) when the user last left this entry; restored on return.
    pub scroll: (i32, i32),
    /// Whether back/forward may re-navigate here. A POST result is not safely re-navigable
    /// (it would re-submit the form body). The engine is GET-only today, so every entry is
    /// navigable; this is the hook for when form submissions arrive.
    pub navigable: bool,
    parent: Option<HistoryEntryId>,
    children: Vec<HistoryEntryId>,
    /// Which child to follow on a plain "forward": the most recently visited one.
    preferred_child: Option<HistoryEntryId>,
}

/// Embedder-facing snapshot of the tree, sent with every change so shells never have to
/// query. `forward` lists the current entry's navigable children, preferred first.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HistorySnapshot {
    pub current: Option<HistoryEntryId>,
    pub can_go_back: bool,
    pub forward: Vec<HistoryEntrySummary>,
    /// Every entry, in creation order (index == `HistoryEntryId.0`); lets a shell render a
    /// full history view without further round-trips.
    pub entries: Vec<HistoryEntrySummary>,
}

/// The public part of a [`HistoryEntry`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntrySummary {
    pub id: HistoryEntryId,
    pub url: Url,
    pub title: Option<String>,
    pub parent: Option<HistoryEntryId>,
}

/// Tree-structured session history for one tab.
#[derive(Debug, Clone, Default)]
pub struct History {
    entries: Vec<HistoryEntry>,
    current: Option<HistoryEntryId>,
}

impl History {
    /// Record a committed navigation to a new page: append a child of the current entry and
    /// move to it. When the current entry already has children this forks a new branch.
    pub fn push(&mut self, url: Url, title: Option<String>) -> HistoryEntryId {
        let id = HistoryEntryId(self.entries.len());
        self.entries.push(HistoryEntry {
            url,
            title,
            scroll: (0, 0),
            navigable: true,
            parent: self.current,
            children: Vec::new(),
            preferred_child: None,
        });
        if let Some(cur) = self.current {
            let parent = &mut self.entries[cur.0];
            parent.children.push(id);
            parent.preferred_child = Some(id);
        }
        self.current = Some(id);
        id
    }

    /// Replace the current entry's URL in place (server redirects, fragment changes that
    /// should not create an entry). No-op without a current entry.
    pub fn replace_current_url(&mut self, url: Url) {
        if let Some(cur) = self.current {
            self.entries[cur.0].url = url;
        }
    }

    pub fn current(&self) -> Option<HistoryEntryId> {
        self.current
    }

    pub fn current_entry(&self) -> Option<&HistoryEntry> {
        self.current.map(|id| &self.entries[id.0])
    }

    pub fn entry(&self, id: HistoryEntryId) -> Option<&HistoryEntry> {
        self.entries.get(id.0)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Update the current entry's title (once the document's `<title>` is known).
    pub fn set_current_title(&mut self, title: Option<String>) {
        if let Some(cur) = self.current {
            self.entries[cur.0].title = title;
        }
    }

    /// Remember the scroll offset of the current entry (call before leaving it).
    pub fn set_current_scroll(&mut self, x: i32, y: i32) {
        if let Some(cur) = self.current {
            self.entries[cur.0].scroll = (x, y);
        }
    }

    /// Nearest navigable ancestor of the current entry.
    fn back_target(&self) -> Option<HistoryEntryId> {
        let mut node = self.entries[self.current?.0].parent;
        while let Some(id) = node {
            if self.entries[id.0].navigable {
                return Some(id);
            }
            node = self.entries[id.0].parent;
        }
        None
    }

    /// Navigable children of the current entry, preferred child first.
    pub fn forward_targets(&self) -> Vec<HistoryEntryId> {
        let Some(cur) = self.current else {
            return Vec::new();
        };
        let entry = &self.entries[cur.0];
        let mut out: Vec<HistoryEntryId> = entry
            .children
            .iter()
            .copied()
            .filter(|c| self.entries[c.0].navigable)
            .collect();
        if let Some(pref) = entry.preferred_child {
            if let Some(pos) = out.iter().position(|c| *c == pref) {
                out.swap(0, pos);
            }
        }
        out
    }

    pub fn can_go_back(&self) -> bool {
        self.back_target().is_some()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward_targets().is_empty()
    }

    /// Move to the nearest navigable ancestor. Returns the entry moved to.
    pub fn go_back(&mut self) -> Option<HistoryEntryId> {
        let id = self.back_target()?;
        self.current = Some(id);
        Some(id)
    }

    /// Move to `entry` if it is a navigable forward child of the current entry, or to the
    /// preferred forward child when `entry` is `None`. Choosing a branch makes it the
    /// preferred one, so a later plain forward retraces the user's last choice. Returns the
    /// entry moved to.
    pub fn go_forward(&mut self, entry: Option<HistoryEntryId>) -> Option<HistoryEntryId> {
        let targets = self.forward_targets();
        let id = match entry {
            Some(id) if targets.contains(&id) => id,
            Some(_) => return None,
            None => *targets.first()?,
        };
        if let Some(cur) = self.current {
            self.entries[cur.0].preferred_child = Some(id);
        }
        self.current = Some(id);
        Some(id)
    }

    /// Jump to any navigable entry in the tree (a full history view picking an entry).
    /// Marks the path from its parent as preferred so a later "forward" retraces it.
    pub fn go_to(&mut self, id: HistoryEntryId) -> Option<HistoryEntryId> {
        let entry = self.entries.get(id.0)?;
        if !entry.navigable {
            return None;
        }
        if let Some(parent) = entry.parent {
            self.entries[parent.0].preferred_child = Some(id);
        }
        self.current = Some(id);
        Some(id)
    }

    pub fn snapshot(&self) -> HistorySnapshot {
        let summary = |id: HistoryEntryId| {
            let e = &self.entries[id.0];
            HistoryEntrySummary {
                id,
                url: e.url.clone(),
                title: e.title.clone(),
                parent: e.parent,
            }
        };
        HistorySnapshot {
            current: self.current,
            can_go_back: self.can_go_back(),
            forward: self.forward_targets().into_iter().map(summary).collect(),
            entries: (0..self.entries.len()).map(|i| summary(HistoryEntryId(i))).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn empty_history_cannot_move() {
        let mut h = History::default();
        assert!(!h.can_go_back());
        assert!(!h.can_go_forward());
        assert_eq!(h.go_back(), None);
        assert_eq!(h.go_forward(None), None);
        assert_eq!(h.current(), None);
    }

    #[test]
    fn push_then_back_and_forward() {
        let mut h = History::default();
        let a = h.push(u("https://a/"), None);
        let b = h.push(u("https://b/"), None);
        assert_eq!(h.current(), Some(b));
        assert!(h.can_go_back());
        assert!(!h.can_go_forward());

        assert_eq!(h.go_back(), Some(a));
        assert!(!h.can_go_back());
        assert!(h.can_go_forward());
        assert_eq!(h.forward_targets(), vec![b]);

        assert_eq!(h.go_forward(None), Some(b));
        assert!(!h.can_go_forward());
    }

    #[test]
    fn navigating_away_forks_instead_of_discarding() {
        let mut h = History::default();
        let a = h.push(u("https://a/"), None);
        let b = h.push(u("https://b/"), None);
        h.go_back();
        let c = h.push(u("https://c/"), None);

        // Both branches survive; the newest is preferred.
        h.go_back();
        assert_eq!(h.current(), Some(a));
        assert_eq!(h.forward_targets(), vec![c, b]);
        assert_eq!(h.go_forward(None), Some(c));

        // Explicit choice of the older branch.
        h.go_back();
        assert_eq!(h.go_forward(Some(b)), Some(b));
        // Choosing it makes it preferred next time.
        h.go_back();
        assert_eq!(h.forward_targets(), vec![b, c]);
    }

    #[test]
    fn go_forward_rejects_non_children() {
        let mut h = History::default();
        let a = h.push(u("https://a/"), None);
        let _b = h.push(u("https://b/"), None);
        // `a` is the parent, not a forward child.
        assert_eq!(h.go_forward(Some(a)), None);
        assert_eq!(h.go_forward(Some(HistoryEntryId(99))), None);
    }

    #[test]
    fn go_to_jumps_anywhere_and_sets_preference() {
        let mut h = History::default();
        let a = h.push(u("https://a/"), None);
        let b = h.push(u("https://b/"), None);
        let _c = h.push(u("https://c/"), None);
        assert_eq!(h.go_to(a), Some(a));
        assert_eq!(h.current(), Some(a));
        assert_eq!(h.forward_targets(), vec![b]);
        assert_eq!(h.go_to(HistoryEntryId(42)), None);
    }

    #[test]
    fn scroll_and_title_are_per_entry() {
        let mut h = History::default();
        let a = h.push(u("https://a/"), Some("A".into()));
        h.set_current_scroll(0, 500);
        let b = h.push(u("https://b/"), None);
        h.set_current_title(Some("B".into()));
        assert_eq!(h.entry(a).unwrap().scroll, (0, 500));
        assert_eq!(h.entry(b).unwrap().scroll, (0, 0));
        assert_eq!(h.entry(b).unwrap().title.as_deref(), Some("B"));
    }

    #[test]
    fn non_navigable_entries_are_skipped_by_back() {
        let mut h = History::default();
        let a = h.push(u("https://a/"), None);
        let post = h.push(u("https://form/"), None);
        h.entries[post.0].navigable = false;
        let _c = h.push(u("https://c/"), None);
        assert_eq!(h.go_back(), Some(a));
    }

    #[test]
    fn snapshot_reflects_tree() {
        let mut h = History::default();
        let a = h.push(u("https://a/"), Some("A".into()));
        let b = h.push(u("https://b/"), None);
        h.go_back();
        let s = h.snapshot();
        assert_eq!(s.current, Some(a));
        assert!(!s.can_go_back);
        assert_eq!(s.forward.len(), 1);
        assert_eq!(s.forward[0].id, b);
        assert_eq!(s.entries.len(), 2);
        assert_eq!(s.entries[1].parent, Some(a));
        assert_eq!(s.entries[0].title.as_deref(), Some("A"));
    }

    #[test]
    fn replace_current_url_keeps_position() {
        let mut h = History::default();
        let a = h.push(u("https://a/"), None);
        h.replace_current_url(u("https://a/final"));
        assert_eq!(h.entry(a).unwrap().url, u("https://a/final"));
        assert_eq!(h.current(), Some(a));
    }
}
