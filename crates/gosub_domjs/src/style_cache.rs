//! One cascade per element, not one per property read.
//!
//! `getComputedStyle(el)` hands JavaScript a proxy, and the proxy turns every property access
//! into its own `getPropertyValue` call. Each of those used to run the whole cascade for the
//! element *and every element ancestor* - so a script reading twenty properties off an element
//! forty deep in the tree asked the cascade eight hundred times for the same answer. This keeps
//! the resolved map of each element, so the second read of the same element is a lookup.
//!
//! The cache is correct by being cheap to throw away: any DOM mutation clears it whole, and a
//! read checks that it still belongs to this document, to the same stylesheets and to the same
//! media environment before trusting it. Per-node invalidation would be finer, but a script
//! that mutates and re-reads in a loop is rare next to one that reads a stack of properties,
//! and being coarse is what makes it easy to see that nothing stale can survive.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use gosub_css3::matcher::styling::CssProperties;
use gosub_css3::media_query::{media_environment, MediaEnvironment};
use gosub_css3::system::Css3System;
use gosub_interface::css3::{CssPropertyMap as _, CssSystem as _};
use gosub_interface::document::Document as _;
use gosub_interface::node::NodeType;
use gosub_shared::node::NodeId;

use crate::{Doc, DomConfig};

/// What the entries were resolved against, and the entries.
#[derive(Default)]
struct StyleCache {
    /// The document, by address, which tells apart the documents that are alive at once - node
    /// ids mean nothing across them. It cannot tell a document from a later one the allocator
    /// put at the same address, so [`invalidate`] is called when one is parsed.
    document: usize,
    /// How many stylesheets it had, so a sheet added afterwards is noticed.
    sheets: usize,
    /// The media environment the cascade read. A resize or a colour-scheme change can flip an
    /// `@media` condition or move what a `vw` means, and neither goes through a DOM mutation.
    environment: Option<MediaEnvironment>,
    elements: HashMap<NodeId, Arc<CssProperties>>,
    /// `::before` / `::after`, which are keyed by their originating element and name. `None` is
    /// an answer too: it says the element generates no such box.
    pseudos: HashMap<(NodeId, String), Option<Arc<CssProperties>>>,
}

thread_local! {
    static CACHE: RefCell<StyleCache> = RefCell::new(StyleCache::default());
}

/// Throw the cache away. Called from every binding that changes the document.
pub(crate) fn invalidate() {
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.elements.clear();
        cache.pseudos.clear();
    });
}

/// Drop the entries when they belong to something other than what is being read now.
fn ensure_current(doc: &Doc) {
    let document = std::ptr::from_ref(doc) as usize;
    let sheets = doc.stylesheets().len();
    let environment = media_environment();
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.document == document && cache.sheets == sheets && cache.environment == Some(environment) {
            return;
        }
        cache.document = document;
        cache.sheets = sheets;
        cache.environment = Some(environment);
        cache.elements.clear();
        cache.pseudos.clear();
    });
}

fn cached(id: NodeId) -> Option<Arc<CssProperties>> {
    CACHE.with(|cache| cache.borrow().elements.get(&id).map(Arc::clone))
}

/// Run the cascade for one element and resolve every value it settled, so the elements below it
/// inherit computed values rather than cascaded ones.
fn resolve(doc: &Doc, id: NodeId, parent: Option<&CssProperties>) -> Option<CssProperties> {
    let mut map = Css3System::properties_from_node::<DomConfig>(doc, id, doc.stylesheets(), parent)?;
    for (_, property) in map.iter_mut() {
        property.compute_value();
    }
    Some(map)
}

/// The resolved cascade of `id`.
///
/// Styles resolve top-down, so the walk goes up to the nearest ancestor the cache already has -
/// or to the root - and comes back down filling in what it passed. An element the cascade has
/// nothing for (a `<script>`, say) contributes no map of its own and is not cached; what comes
/// back for it is the nearest ancestor that resolved, which is what the uncached walk answered
/// before.
pub(crate) fn element_style(doc: &Doc, id: NodeId) -> Option<Arc<CssProperties>> {
    ensure_current(doc);

    let mut chain: Vec<NodeId> = Vec::new();
    let mut resolved: Option<Arc<CssProperties>> = None;
    let mut current = Some(id);
    while let Some(node) = current {
        if doc.node_type(node) == NodeType::ElementNode {
            if let Some(map) = cached(node) {
                resolved = Some(map);
                break;
            }
            chain.push(node);
        }
        current = doc.parent(node);
    }

    for node in chain.into_iter().rev() {
        let Some(map) = resolve(doc, node, resolved.as_deref()) else {
            continue;
        };
        let map = Arc::new(map);
        CACHE.with(|cache| cache.borrow_mut().elements.insert(node, Arc::clone(&map)));
        resolved = Some(map);
    }
    resolved
}

/// The resolved cascade of a `::before` / `::after` on `id`, or `None` when it generates no box.
pub(crate) fn pseudo_style(doc: &Doc, id: NodeId, pseudo: &str) -> Option<Arc<CssProperties>> {
    ensure_current(doc);
    let key = (id, pseudo.to_string());
    if let Some(entry) = CACHE.with(|cache| cache.borrow().pseudos.get(&key).cloned()) {
        return entry;
    }

    let owner = element_style(doc, id);
    let map =
        Css3System::pseudo_properties_from_node::<DomConfig>(doc, id, doc.stylesheets(), pseudo, owner.as_deref()).map(
            |mut map| {
                for (_, property) in map.iter_mut() {
                    property.compute_value();
                }
                Arc::new(map)
            },
        );
    CACHE.with(|cache| cache.borrow_mut().pseudos.insert(key, map.clone()));
    map
}
