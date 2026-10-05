//! The order cascade layers sort in, worked out across a document's stylesheets.
//!
//! A layer is named in one sheet and may be filled in by another, so its place cannot be settled
//! while a single sheet is parsed: it belongs to the origin, not to the file. Each sheet records
//! the layers it declares, in the order it declares them ([`CssStylesheet::layers`]), and this
//! merges those lists per origin.

use std::collections::HashMap;

use crate::stylesheet::CssStylesheet;
use gosub_interface::css3::CssOrigin;

/// One layer's place among its siblings, and its sub-layers under it.
#[derive(Default)]
struct Node {
    /// Sub-layers, in the order they were first declared.
    children: Vec<(String, Node)>,
}

impl Node {
    fn child(&mut self, name: &str) -> &mut Node {
        if let Some(at) = self.children.iter().position(|(known, _)| known == name) {
            return &mut self.children[at].1;
        }
        self.children.push((name.to_string(), Node::default()));
        let last = self.children.len() - 1;
        &mut self.children[last].1
    }

    /// Number every layer in this subtree, weakest first.
    ///
    /// Sub-layers come before the layer that holds them, because a layer's own rules are
    /// unlayered *within* it and so beat anything it nests (css-cascade-5 §6.4.1 applied one
    /// level down). Siblings keep declaration order, weakest first.
    fn flatten(&self, prefix: &str, next: &mut u32, out: &mut HashMap<String, u32>) {
        for (name, child) in &self.children {
            let full = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}.{name}")
            };
            child.flatten(&full, next, out);
            out.insert(full, *next);
            *next += 1;
        }
    }
}

/// An origin as a map key. `CssOrigin` is not hashable, and there are only ever three.
fn origin_key(origin: CssOrigin) -> u8 {
    match origin {
        CssOrigin::UserAgent => 0,
        CssOrigin::Author => 1,
        CssOrigin::User => 2,
    }
}

/// Where every layer of every origin sorts, by full dotted name.
#[derive(Default)]
pub struct LayerOrder {
    ranks: HashMap<(u8, String), u32>,
}

impl LayerOrder {
    /// Merge the layer lists of `sheets` into one order per origin.
    ///
    /// `None` when no sheet declares a layer at all, which is the usual case and lets the
    /// cascade skip the lookup entirely.
    #[must_use]
    pub fn build(sheets: &[&CssStylesheet]) -> Option<Self> {
        if sheets.iter().all(|sheet| sheet.layers.is_empty()) {
            return None;
        }
        let mut trees: HashMap<u8, Node> = HashMap::new();
        for sheet in sheets {
            let tree = trees.entry(origin_key(sheet.origin)).or_default();
            for name in &sheet.layers {
                let mut node = &mut *tree;
                for part in name.split('.') {
                    node = node.child(part);
                }
            }
        }
        let mut ranks = HashMap::new();
        for (origin, tree) in &trees {
            let mut flat = HashMap::new();
            let mut next = 0;
            tree.flatten("", &mut next, &mut flat);
            ranks.extend(flat.into_iter().map(|(name, rank)| ((*origin, name), rank)));
        }
        Some(Self { ranks })
    }

    /// The rank of a named layer: higher means declared later, which for a normal declaration
    /// means it wins. A name this does not know has never been declared and sorts weakest.
    #[must_use]
    pub fn rank(&self, origin: CssOrigin, name: &str) -> u32 {
        self.ranks
            .get(&(origin_key(origin), name.to_string()))
            .copied()
            .unwrap_or(0)
    }
}
