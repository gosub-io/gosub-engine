//! The stack of open elements, with a cached answer to "is there a `p` in button scope".
//!
//! Every block start tag asks that question before it is inserted. That covers `<div>`, `<li>`,
//! `<ul>`, `<h1>` and many more. Answering it walks down the stack until it finds a `p` or a
//! scope boundary. A page that nests thousands of blocks has no `p` and no boundary, so each walk
//! went to the bottom, and parsing was quadratic in depth. The tree depth cap does not help here,
//! because it limits the tree and not the stack.
//!
//! Blink keeps one bit per stack entry for this. It is `has_p_element_in_button_scope_`, kept up
//! to date by `HTMLElementStack::UpdatePElementInButtonScope`. The bit of an entry depends only
//! on that entry and the entries below it, so it can be computed bottom-up. Here the bits are
//! filled in lazily when asked for. Every change to the stack drops the bits from the lowest
//! changed position upward. Pushes and pops make up almost all changes, and they keep the rest.

use core::cell::RefCell;
use core::ops::{Deref, Index, IndexMut};

use gosub_shared::node::NodeId;

#[derive(Default)]
pub(crate) struct OpenElements {
    items: Vec<NodeId>,
    /// `p_in_button_scope[i]` is the answer when the stack is `items[..=i]`. Only a prefix is
    /// known at any time. It is filled in on demand.
    p_in_button_scope: RefCell<Vec<bool>>,
}

#[cfg(test)]
thread_local! {
    /// How many stack entries the cached query has looked at on this thread. Tests use it to
    /// check that the cost grows with the document and not with its square.
    pub(crate) static STEPS: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
}

impl OpenElements {
    pub(crate) fn push(&mut self, id: NodeId) {
        self.items.push(id);
    }

    pub(crate) fn pop(&mut self) -> Option<NodeId> {
        let id = self.items.pop();
        self.forget_from(self.items.len());
        id
    }

    pub(crate) fn truncate(&mut self, len: usize) {
        self.items.truncate(len);
        self.forget_from(len);
    }

    #[cfg(test)]
    pub(crate) fn clear(&mut self) {
        self.truncate(0);
    }

    pub(crate) fn remove(&mut self, index: usize) -> NodeId {
        self.forget_from(index);
        self.items.remove(index)
    }

    pub(crate) fn insert(&mut self, index: usize, id: NodeId) {
        self.forget_from(index);
        self.items.insert(index, id);
    }

    pub(crate) fn retain(&mut self, mut keep: impl FnMut(&NodeId) -> bool) {
        // Everything above the first removed entry moves down, so its bits are stale.
        let mut index = 0;
        let mut first_removed = None;
        self.items.retain(|id| {
            let kept = keep(id);
            if !kept && first_removed.is_none() {
                first_removed = Some(index);
            }
            index += 1;
            kept
        });
        if let Some(index) = first_removed {
            self.forget_from(index);
        }
    }

    /// Whether there is an HTML `p` in button scope. `step` classifies one entry the way the
    /// scope walk does. It returns `Some(true)` for the target, `Some(false)` for a scope
    /// boundary, and `None` to keep looking further down.
    pub(crate) fn has_p_in_button_scope(&self, step: impl Fn(NodeId) -> Option<bool>) -> bool {
        let mut known = self.p_in_button_scope.borrow_mut();
        while known.len() < self.items.len() {
            #[cfg(test)]
            STEPS.with(|n| n.set(n.get() + 1));
            let below = known.last().copied().unwrap_or(false);
            let bit = step(self.items[known.len()]).unwrap_or(below);
            known.push(bit);
        }
        known.last().copied().unwrap_or(false)
    }

    fn forget_from(&self, index: usize) {
        self.p_in_button_scope.borrow_mut().truncate(index);
    }
}

impl Deref for OpenElements {
    type Target = [NodeId];

    fn deref(&self) -> &[NodeId] {
        &self.items
    }
}

impl<'a> IntoIterator for &'a OpenElements {
    type Item = &'a NodeId;
    type IntoIter = core::slice::Iter<'a, NodeId>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.iter()
    }
}

impl Index<usize> for OpenElements {
    type Output = NodeId;

    fn index(&self, index: usize) -> &NodeId {
        &self.items[index]
    }
}

impl IndexMut<usize> for OpenElements {
    fn index_mut(&mut self, index: usize) -> &mut NodeId {
        self.forget_from(index);
        &mut self.items[index]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Entries 1..=9. Entry 3 is a `p`, entry 6 is a boundary, the rest are neither.
    fn step(id: NodeId) -> Option<bool> {
        match usize::from(id) {
            3 => Some(true),
            6 => Some(false),
            _ => None,
        }
    }

    fn stack(ids: &[usize]) -> OpenElements {
        let mut s = OpenElements::default();
        for &id in ids {
            s.push(NodeId::from(id));
        }
        s
    }

    /// The same walk the parser does, from the top down, without the cache.
    fn walk(s: &OpenElements) -> bool {
        s.iter().rev().find_map(|&id| step(id)).unwrap_or(false)
    }

    #[test]
    fn matches_a_plain_walk_through_every_kind_of_change() {
        let mut s = stack(&[1, 2, 3, 4, 5]);
        assert!(s.has_p_in_button_scope(step));
        s.push(NodeId::from(6usize));
        assert_eq!(s.has_p_in_button_scope(step), walk(&s));
        s.pop();
        assert_eq!(s.has_p_in_button_scope(step), walk(&s));
        s.remove(2);
        assert_eq!(s.has_p_in_button_scope(step), walk(&s));
        s.insert(1, NodeId::from(3usize));
        assert_eq!(s.has_p_in_button_scope(step), walk(&s));
        s[1] = NodeId::from(7usize);
        assert_eq!(s.has_p_in_button_scope(step), walk(&s));
        s.push(NodeId::from(3usize));
        s.push(NodeId::from(6usize));
        s.push(NodeId::from(8usize));
        assert_eq!(s.has_p_in_button_scope(step), walk(&s));
        s.retain(|&id| usize::from(id) != 6);
        assert_eq!(s.has_p_in_button_scope(step), walk(&s));
        s.truncate(1);
        assert_eq!(s.has_p_in_button_scope(step), walk(&s));
        s.clear();
        assert!(!s.has_p_in_button_scope(step));
    }

    #[test]
    fn a_push_then_a_query_looks_at_one_entry() {
        let mut s = OpenElements::default();
        for id in 0..10_000usize {
            s.push(NodeId::from(10 + id));
            let before = STEPS.with(|n| n.get());
            assert!(!s.has_p_in_button_scope(step));
            assert_eq!(STEPS.with(|n| n.get()) - before, 1);
        }
    }

    /// Through the parser. Every `<div>` asks whether a `p` is in button scope, and 20,000
    /// nested ones used to walk the whole stack each time, some 200 million steps. Now each
    /// entry is looked at about once.
    #[test]
    fn parsing_nested_blocks_is_linear_in_depth() {
        use crate::document::document_impl::DocumentImpl;
        use crate::parser::Html5Parser;
        use gosub_css3::system::Css3System;
        use gosub_interface::config::ModuleConfiguration;

        #[derive(Clone, Debug, PartialEq)]
        struct Config;
        impl ModuleConfiguration for Config {
            type CssSystem = Css3System;
            type Document = DocumentImpl<Self>;
            type HtmlParser = Html5Parser<'static, Self>;
        }

        const DEPTH: usize = 20_000;
        for (open, close) in [("<div>", "</div>"), ("<ul><li>", "</li></ul>")] {
            let html = format!(
                "<html><body>{}x{}</body></html>",
                open.repeat(DEPTH),
                close.repeat(DEPTH)
            );
            STEPS.with(|n| n.set(0));
            let _ = crate::html_compile::<Config>(&html);
            let steps = STEPS.with(|n| n.get());
            assert!(steps <= 4 * DEPTH, "{open}: {steps} scope steps for {DEPTH} levels");
        }
    }
}
