//! `EventTarget` listener lists and the `AbortSignal` dependency graph as
//! described by <https://dom.spec.whatwg.org/>. JS-side values (callbacks,
//! abort reasons, event objects) stay with the embedder, referenced here by
//! opaque `u64` keys; the dispatch loop is driven from the embedder too.

use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ListenerEntry {
    id: u64,
    event_type: String,
    callback: u64,
    capture: bool,
    passive: bool,
    once: bool,
}

#[derive(Debug, Default)]
struct SignalEntry {
    aborted: bool,
    /// Dependent signal: the source signals it follows
    sources: Vec<u64>,
    /// Source signal: dependents in creation order (= abort event fire order)
    dependents: Vec<u64>,
    /// Listeners to remove when this signal aborts
    listener_links: Vec<(u64, u64)>,
}

/// Signals newly aborted by an `abort` call, in abort event fire order
pub type AbortOrder = Vec<u64>;

#[derive(Debug, Default)]
pub struct EventsHost {
    targets: HashMap<u64, Vec<ListenerEntry>>,
    signals: HashMap<u64, SignalEntry>,
    next_target: u64,
    next_listener: u64,
    next_signal: u64,
}

impl EventsHost {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn new_target(&mut self) -> u64 {
        self.next_target += 1;
        self.targets.insert(self.next_target, Vec::new());
        self.next_target
    }

    /// Add a listener; `None` when one with the same identity (type, callback,
    /// capture — `once`/`passive` don't count) already exists
    pub fn add_listener(
        &mut self,
        target: u64,
        event_type: &str,
        callback: u64,
        capture: bool,
        passive: bool,
        once: bool,
    ) -> Option<u64> {
        let list = self.targets.entry(target).or_default();
        if list
            .iter()
            .any(|l| l.event_type == event_type && l.callback == callback && l.capture == capture)
        {
            return None;
        }
        self.next_listener += 1;
        let id = self.next_listener;
        list.push(ListenerEntry {
            id,
            event_type: event_type.to_owned(),
            callback,
            capture,
            passive,
            once,
        });
        Some(id)
    }

    pub fn remove_listener(&mut self, target: u64, event_type: &str, callback: u64, capture: bool) {
        if let Some(list) = self.targets.get_mut(&target) {
            list.retain(|l| !(l.event_type == event_type && l.callback == callback && l.capture == capture));
        }
    }

    /// The listener-list clone dispatch iterates: listeners added during
    /// dispatch don't run for the in-flight event
    #[must_use]
    pub fn snapshot(&self, target: u64, event_type: &str) -> Vec<u64> {
        self.targets
            .get(&target)
            .map(|list| {
                list.iter()
                    .filter(|l| l.event_type == event_type)
                    .map(|l| l.id)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Resolve a snapshot entry to (callback, passive), or `None` if it was
    /// removed since the snapshot. A `once` listener is removed here, before
    /// its callback runs, so a nested dispatch can't re-enter it.
    pub fn begin_invoke(&mut self, target: u64, listener: u64) -> Option<(u64, bool)> {
        let list = self.targets.get_mut(&target)?;
        let pos = list.iter().position(|l| l.id == listener)?;
        let (callback, passive, once) = (list[pos].callback, list[pos].passive, list[pos].once);
        if once {
            list.remove(pos);
        }
        Some((callback, passive))
    }

    pub fn new_signal(&mut self) -> u64 {
        self.next_signal += 1;
        self.signals.insert(self.next_signal, SignalEntry::default());
        self.next_signal
    }

    #[must_use]
    pub fn is_aborted(&self, signal: u64) -> bool {
        self.signals.get(&signal).is_some_and(|s| s.aborted)
    }

    /// Create a dependent signal (`AbortSignal.any`); dependents link to
    /// non-dependent signals only. If a source is already aborted the new
    /// signal starts aborted and that source is returned for reason copying.
    pub fn new_dependent(&mut self, sources: &[u64]) -> (u64, Option<u64>) {
        let id = self.new_signal();

        if let Some(&aborted) = sources.iter().find(|s| self.is_aborted(**s)) {
            if let Some(entry) = self.signals.get_mut(&id) {
                entry.aborted = true;
            }
            return (id, Some(aborted));
        }

        let mut flattened: Vec<u64> = Vec::new();
        for &source in sources {
            let Some(entry) = self.signals.get(&source) else {
                continue;
            };
            let roots = if entry.sources.is_empty() {
                vec![source]
            } else {
                entry.sources.clone()
            };
            for root in roots {
                if !flattened.contains(&root) {
                    flattened.push(root);
                }
            }
        }

        for &root in &flattened {
            if let Some(entry) = self.signals.get_mut(&root) {
                entry.dependents.push(id);
            }
        }
        if let Some(entry) = self.signals.get_mut(&id) {
            entry.sources = flattened;
        }
        (id, None)
    }

    /// Register a listener to remove when `signal` aborts
    pub fn link_listener(&mut self, signal: u64, target: u64, listener: u64) {
        if let Some(entry) = self.signals.get_mut(&signal) {
            entry.listener_links.push((target, listener));
        }
    }

    /// Mark a signal and its not-yet-aborted dependents aborted, remove their
    /// linked listeners, and return the abort-event fire order
    pub fn abort(&mut self, signal: u64) -> AbortOrder {
        let newly: Vec<u64> = {
            let Some(entry) = self.signals.get(&signal) else {
                return Vec::new();
            };
            if entry.aborted {
                return Vec::new();
            }
            let mut newly = vec![signal];
            newly.extend(entry.dependents.iter().copied().filter(|d| !self.is_aborted(*d)));
            newly
        };

        for &id in &newly {
            let Some(entry) = self.signals.get_mut(&id) else {
                continue;
            };
            entry.aborted = true;
            for (target, listener) in std::mem::take(&mut entry.listener_links) {
                if let Some(list) = self.targets.get_mut(&target) {
                    list.retain(|l| l.id != listener);
                }
            }
        }
        newly
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_dedup_and_remove() {
        let mut h = EventsHost::new();
        let t = h.new_target();
        let id = h.add_listener(t, "click", 1, false, false, false).unwrap();
        // Same identity — even with different once/passive — is not re-added
        assert_eq!(h.add_listener(t, "click", 1, false, true, true), None);
        // Different capture or type is a different listener
        assert!(h.add_listener(t, "click", 1, true, false, false).is_some());
        assert!(h.add_listener(t, "hover", 1, false, false, false).is_some());

        h.remove_listener(t, "click", 1, false);
        assert_eq!(h.begin_invoke(t, id), None);
        // Can be re-added after removal
        assert!(h.add_listener(t, "click", 1, false, false, false).is_some());
    }

    #[test]
    fn snapshot_does_not_see_later_additions() {
        let mut h = EventsHost::new();
        let t = h.new_target();
        let a = h.add_listener(t, "x", 1, false, false, false).unwrap();
        let snap = h.snapshot(t, "x");
        let b = h.add_listener(t, "x", 2, false, false, false).unwrap();
        assert_eq!(snap, vec![a]);
        assert_eq!(h.snapshot(t, "x"), vec![a, b]);
    }

    #[test]
    fn once_is_removed_before_invoke() {
        let mut h = EventsHost::new();
        let t = h.new_target();
        let id = h.add_listener(t, "x", 1, false, true, true).unwrap();
        assert_eq!(h.begin_invoke(t, id), Some((1, true)));
        // Second resolve (e.g. from a nested dispatch snapshot) finds nothing
        assert_eq!(h.begin_invoke(t, id), None);
        assert!(h.snapshot(t, "x").is_empty());
    }

    #[test]
    fn abort_marks_dependents_in_creation_order() {
        let mut h = EventsHost::new();
        let source = h.new_signal();
        let (dep1, none1) = h.new_dependent(&[source]);
        let (dep2, none2) = h.new_dependent(&[source]);
        // Dependent-of-dependent links to the root source, keeping global order
        let (dep3, none3) = h.new_dependent(&[dep1]);
        assert_eq!((none1, none2, none3), (None, None, None));

        assert_eq!(h.abort(source), vec![source, dep1, dep2, dep3]);
        assert!(h.is_aborted(dep3));
        // Aborting again is a no-op
        assert!(h.abort(source).is_empty());
        assert!(h.abort(dep1).is_empty());
    }

    #[test]
    fn dependent_of_aborted_source_is_born_aborted() {
        let mut h = EventsHost::new();
        let alive = h.new_signal();
        let dead1 = h.new_signal();
        let dead2 = h.new_signal();
        h.abort(dead1);
        h.abort(dead2);

        // The first aborted source in argument order wins
        let (dep, from) = h.new_dependent(&[alive, dead1, dead2]);
        assert!(h.is_aborted(dep));
        assert_eq!(from, Some(dead1));
        // A born-aborted dependent never fires: it is not in anyone's list
        assert_eq!(h.abort(alive), vec![alive]);
    }

    #[test]
    fn duplicate_sources_are_linked_once() {
        let mut h = EventsHost::new();
        let source = h.new_signal();
        let (dep, _) = h.new_dependent(&[source, source]);
        assert_eq!(h.abort(source), vec![source, dep]);
    }

    #[test]
    fn abort_removes_linked_listeners() {
        let mut h = EventsHost::new();
        let t = h.new_target();
        let signal = h.new_signal();
        let (dep, _) = h.new_dependent(&[signal]);
        let a = h.add_listener(t, "x", 1, false, false, false).unwrap();
        let b = h.add_listener(t, "y", 2, false, false, false).unwrap();
        h.link_listener(signal, t, a);
        h.link_listener(dep, t, b);

        h.abort(signal);
        // Both the signal's and its dependent's linked listeners are gone,
        // visible mid-dispatch through begin_invoke
        assert_eq!(h.begin_invoke(t, a), None);
        assert_eq!(h.begin_invoke(t, b), None);
    }
}
