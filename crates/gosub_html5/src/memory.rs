//! What a parsed document costs, as rows for the shared memory report.
//!
//! Three rows rather than one, because "how much does a DOM use" has three different answers
//! depending on the page: the nodes themselves (fixed per node, and the arena's own slots),
//! what elements carry (names, attributes, classes), and the text.

use gosub_interface::config::HasDocument;
use gosub_shared::memory::{record, HeapSize, Row, Walk};

use crate::document::document_impl::DocumentImpl;
use crate::node::data::comment::CommentData;
use crate::node::data::doctype::DocTypeData;
use crate::node::data::document::DocumentData;
use crate::node::data::element::ElementData;
use crate::node::data::shadow_root::ShadowRootData;
use crate::node::data::text::TextData;
use crate::node::node_impl::{NodeDataTypeInternal, NodeImpl};

impl HeapSize for TextData {
    fn heap_size(&self, walk: &mut Walk) {
        self.value.heap_size(walk);
    }
}

impl HeapSize for CommentData {
    fn heap_size(&self, walk: &mut Walk) {
        self.value.heap_size(walk);
    }
}

impl HeapSize for DocTypeData {
    fn heap_size(&self, walk: &mut Walk) {
        self.name.heap_size(walk);
        self.pub_identifier.heap_size(walk);
        self.sys_identifier.heap_size(walk);
    }
}

impl HeapSize for DocumentData {
    fn heap_size(&self, _walk: &mut Walk) {}
}

impl HeapSize for ShadowRootData {
    fn heap_size(&self, _walk: &mut Walk) {}
}

impl HeapSize for ElementData {
    fn heap_size(&self, walk: &mut Walk) {
        self.name.heap_size(walk);
        self.namespace.heap_size(walk);
        self.attributes.heap_size(walk);
        self.class_list.heap_size(walk);
    }
}

impl HeapSize for NodeDataTypeInternal {
    fn heap_size(&self, walk: &mut Walk) {
        match self {
            NodeDataTypeInternal::Document(data) => data.heap_size(walk),
            NodeDataTypeInternal::DocType(data) => data.heap_size(walk),
            NodeDataTypeInternal::Text(data) => data.heap_size(walk),
            NodeDataTypeInternal::Comment(data) => data.heap_size(walk),
            NodeDataTypeInternal::Element(data) => data.heap_size(walk),
            NodeDataTypeInternal::ShadowRoot(data) => data.heap_size(walk),
        }
    }
}

/// Everything a node owns, its payload included.
impl HeapSize for NodeImpl {
    fn heap_size(&self, walk: &mut Walk) {
        self.children.heap_size(walk);
        self.data.heap_size(walk);
    }
}

/// Add this document's rows to the current snapshot.
///
/// The walk is handed in so the whole snapshot shares one set of already-counted allocations:
/// stylesheets and style maps reached from elsewhere are not counted twice.
pub fn record_document<C: HasDocument>(doc: &DocumentImpl<C>, walk: &mut Walk) {
    let mut nodes = 0u64;
    let mut elements = 0u64;
    let mut text_nodes = 0u64;

    // Row 1: the nodes. `children` belongs here rather than with the payload - it is the tree
    // shape, which every node has, not something an element or a text node carries.
    for (_, node) in doc.arena.nodes() {
        nodes += 1;
        node.children.heap_size(walk);
        match &node.data {
            NodeDataTypeInternal::Element(_) => elements += 1,
            NodeDataTypeInternal::Text(_) | NodeDataTypeInternal::Comment(_) => text_nodes += 1,
            _ => {}
        }
    }
    let (owned, shared) = walk.take_counts();
    // The arena keeps a slot per id ever issued, so a page that removed nodes pays for the holes,
    // and it grows by doubling, so it pays for the slots past the last id too.
    let slots = doc.arena.slot_count();
    let allocated = doc.arena.slot_capacity();
    record(
        Row::new(
            "dom.nodes",
            nodes,
            allocated * size_of::<Option<NodeImpl>>(),
            owned,
            shared,
        )
        .with_note(format!(
            "{allocated} arena slots at {} B each, {slots} issued, {} still filled",
            size_of::<Option<NodeImpl>>(),
            nodes
        )),
    );

    // Row 2: what elements carry. Tag name, namespace, attribute map, class list.
    for (_, node) in doc.arena.nodes() {
        if let NodeDataTypeInternal::Element(data) = &node.data {
            data.heap_size(walk);
        }
    }
    let (owned, shared) = walk.take_counts();
    record(Row::new("dom.element.data", elements, 0, owned, shared).with_note("names, attributes, classes"));

    // Row 3: the text. Half the nodes on a page, and none of them styled.
    for (_, node) in doc.arena.nodes() {
        match &node.data {
            NodeDataTypeInternal::Text(data) => data.heap_size(walk),
            NodeDataTypeInternal::Comment(data) => data.heap_size(walk),
            _ => {}
        }
    }
    let (owned, shared) = walk.take_counts();
    record(Row::new("dom.text", text_nodes, 0, owned, shared).with_note("text and comment contents"));
}
