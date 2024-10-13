use gosub_html5::document::document_impl::TreeIterator;
use gosub_render_backend::layout::{HasTextLayout, Layout, LayoutTree, Layouter, TextLayout};
use gosub_render_backend::{layout, Size};
use gosub_shared::document::DocumentHandle;
use gosub_shared::node::NodeId;
use gosub_shared::traits::css3::{CssProperty, CssPropertyMap, CssSystem};
use gosub_shared::traits::document::Document;
use gosub_shared::traits::node::NodeData;
use gosub_shared::traits::node::{ElementDataType, Node as DocumentNode, TextDataType};
use gosub_shared::types::Result;
use std::collections::HashMap;
use std::fmt::{Debug, Formatter};

mod desc;

const INLINE_ELEMENTS: [&str; 31] = [
    "a", "abbr", "acronym", "b", "bdo", "big", "br", "button", "cite", "code", "dfn", "em", "i", "img", "input", "kbd",
    "label", "map", "object", "q", "samp", "script", "select", "small", "span", "strong", "sub", "sup", "textarea",
    "tt", "var",
];

/// Map of all declared values for all nodes in the document
#[derive(Debug)]
pub struct RenderTree<L: Layouter, C: CssSystem> {
    pub nodes: HashMap<NodeId, RenderTreeNode<L, C>>,
    pub root: NodeId,
    pub dirty: bool,
    next_id: NodeId,
}

#[allow(unused)]
impl<L: Layouter, C: CssSystem> LayoutTree<L> for RenderTree<L, C> {
    type NodeId = NodeId;
    type Node = RenderTreeNode<L, C>;

    fn children(&self, id: Self::NodeId) -> Option<Vec<Self::NodeId>> {
        self.get_children(id).cloned()
    }

    fn contains(&self, id: &Self::NodeId) -> bool {
        self.nodes.contains_key(id)
    }

    fn child_count(&self, id: Self::NodeId) -> usize {
        self.child_count(id)
    }

    fn parent_id(&self, id: Self::NodeId) -> Option<Self::NodeId> {
        self.get_node(id).and_then(|node| node.parent)
    }

    fn get_cache(&self, id: Self::NodeId) -> Option<&L::Cache> {
        self.get_node(id).map(|node| &node.cache)
    }

    fn get_layout(&self, id: Self::NodeId) -> Option<&L::Layout> {
        self.get_node(id).map(|node| &node.layout)
    }

    fn get_cache_mut(&mut self, id: Self::NodeId) -> Option<&mut L::Cache> {
        self.get_node_mut(id).map(|node| &mut node.cache)
    }

    fn get_layout_mut(&mut self, id: Self::NodeId) -> Option<&mut L::Layout> {
        self.get_node_mut(id).map(|node| &mut node.layout)
    }

    fn set_cache(&mut self, id: Self::NodeId, cache: L::Cache) {
        if let Some(node) = self.get_node_mut(id) {
            node.cache = cache;
        }
    }

    fn set_layout(&mut self, id: Self::NodeId, layout: L::Layout) {
        if let Some(node) = self.get_node_mut(id) {
            node.layout = layout;
        }
    }

    fn style_dirty(&self, id: Self::NodeId) -> bool {
        self.get_node(id).map(|node| node.properties.is_dirty()).unwrap_or(true)
    }

    fn clean_style(&mut self, id: Self::NodeId) {
        if let Some(node) = self.get_node_mut(id) {
            node.properties.make_clean();
        }
    }

    fn get_node(&mut self, id: Self::NodeId) -> Option<&mut Self::Node> {
        self.get_node_mut(id)
    }
}

impl<L: Layouter, C: CssSystem> RenderTree<L, C> {
    // Generates a new render tree with a root node
    pub fn with_capacity(capacity: usize) -> Self {
        let mut tree = Self {
            nodes: HashMap::with_capacity(capacity),
            root: NodeId::root(),
            dirty: false,
            next_id: NodeId::from(1u64),
        };

        tree.insert_node(
            NodeId::root(),
            RenderTreeNode {
                id: NodeId::root(),
                properties: C::PropertyMap::default(),
                children: Vec::new(),
                parent: None,
                name: String::from("root"),
                namespace: None,
                data: RenderNodeData::Document,
                cache: L::Cache::default(),
                layout: L::Layout::default(),
            },
        );

        tree
    }

    /// Returns the root node of the render tree
    pub fn get_root(&self) -> &RenderTreeNode<L, C> {
        self.nodes.get(&self.root).expect("root node")
    }

    /// Returns the node with the given id
    pub fn get_node(&self, id: NodeId) -> Option<&RenderTreeNode<L, C>> {
        self.nodes.get(&id)
    }

    /// Returns a mutable reference to the node with the given id
    pub fn get_node_mut(&mut self, id: NodeId) -> Option<&mut RenderTreeNode<L, C>> {
        self.nodes.get_mut(&id)
    }

    /// Returns the children of the given node
    pub fn get_children(&self, id: NodeId) -> Option<&Vec<NodeId>> {
        self.nodes.get(&id).map(|node| &node.children)
    }

    /// Returns the children of the given node
    pub fn child_count(&self, id: NodeId) -> usize {
        self.nodes.get(&id).map(|node| node.children.len()).unwrap_or(0)
    }

    /// Inserts a new node into the render tree, note that you are responsible for the node id
    /// and the children of the node
    pub fn insert_node(&mut self, id: NodeId, node: RenderTreeNode<L, C>) {
        self.nodes.insert(id, node);
    }

    /// Deletes the node with the given id from the render tree
    pub fn delete_node(&mut self, id: &NodeId) -> Option<(NodeId, RenderTreeNode<L, C>)> {
        // println!("Deleting node: {id:?}");

        if let Some(n) = self.nodes.get(id) {
            let parent = n.parent;

            if let Some(parent) = parent {
                if let Some(parent_node) = self.nodes.get_mut(&parent) {
                    parent_node.children.retain(|child| child != id);
                }
            }

            self.nodes.remove_entry(id)
        } else {
            None
        }
    }

    /// Returns a recursive list of all child node ids for the given node id
    fn get_child_node_ids(&self, node_id: NodeId) -> Vec<NodeId> {
        let mut result = vec![node_id];

        let node = self.nodes.get(&node_id);
        if node.is_none() {
            return result;
        }
        node.expect("node").children.iter().for_each(|child| {
            let mut childs = self.get_child_node_ids(*child);
            result.append(&mut childs);
        });

        result
    }

    /// Mark the given node as dirty, so it will be recalculated
    pub fn mark_dirty(&mut self, node_id: NodeId) {
        if let Some(props) = self.nodes.get_mut(&node_id) {
            props.properties.make_dirty();
        }
    }

    /// Mark the given node as clean, so it will not be recalculated
    pub fn mark_clean(&mut self, node_id: NodeId) {
        if let Some(props) = self.nodes.get_mut(&node_id) {
            props.properties.make_clean();
        }
    }

    /// Retrieves the property for the given node, or None when not found
    pub fn get_property(&self, node_id: NodeId, prop_name: &str) -> Option<&C::Property> {
        let props = self.nodes.get(&node_id)?;

        props.properties.get(prop_name)
    }

    /// Retrieves the value for the given property for the given node, or None when not found
    pub fn get_all_properties(&self, node_id: NodeId) -> Option<&C::PropertyMap> {
        self.nodes.get(&node_id).map(|props| &props.properties)
    }

    /// Generate a render tree from the given document
    pub fn from_document<D: Document<C>>(document: DocumentHandle<D, C>) -> Self {
        let mut render_tree = RenderTree::with_capacity(document.get().node_count());

        render_tree.generate_from(document);

        render_tree
    }

    fn generate_from<D: Document<C>>(&mut self, handle: DocumentHandle<D, C>) {
        // Iterate the complete document tree

        let iter_handle = DocumentHandle::clone(&handle);

        for current_node_id in TreeIterator::new(iter_handle) {
            let doc = handle.get();

            let node = doc.node_by_id(current_node_id).unwrap();

            let Some(properties) = C::properties_from_node(node, doc.stylesheets(), handle.clone(), current_node_id)
            else {
                //we need to remove it  from the parent in the render tree and from the document

                // todo!("unrenderable node");

                continue;
            };

            let data = node.data();

            let render_data = match RenderNodeData::from_node_data(data) {
                ControlFlow::Ok(data) => data,
                ControlFlow::Drop => continue,
                ControlFlow::Error(e) => {
                    log::error!("Failed to create node data for node: {current_node_id:?} ({e}");
                    continue;
                }
            };

            let mut namespace: Option<String> = None;

            let name = match data {
                NodeData::Element(data) => {
                    namespace = Some(data.namespace().to_string());
                    data.name().to_string()
                }
                NodeData::Text(_) => "#text".to_owned(),
                NodeData::Document(_) => "#document".to_owned(),
                _ => String::new(),
            };

            let render_tree_node = RenderTreeNode {
                id: current_node_id,
                properties,
                children: node.children().to_vec(),
                parent: node.parent_id(),
                name, // We might be able to move node into render_tree_node
                namespace,
                data: render_data,
                cache: L::Cache::default(),
                layout: L::Layout::default(),
            };

            self.nodes.insert(current_node_id, render_tree_node);
        }

        self.next_id = handle.get().peek_next_id();

        self.remove_unrenderable_nodes();

        C::inheritance(self);

        if L::COLLAPSE_INLINE {
            self.collapse_inline(self.root);
        }

        // self.print_tree();
    }

    /// Removes all unrenderable nodes from the render tree
    fn remove_unrenderable_nodes(&mut self) {
        // There are more elements that are not renderable, but for now we only remove the most common ones
        let mut delete_list = Vec::new();

        for id in self.nodes.keys() {
            // Check CSS styles and remove if not renderable
            if let Some(prop) = self.get_property(*id, "display") {
                if prop.as_string() == Some("none") {
                    delete_list.append(&mut self.get_child_node_ids(*id));
                    delete_list.push(*id);
                    continue;
                }
            }
        }

        for id in delete_list {
            self.delete_node(&id);
        }
    }

    /// Collapse all inline elements / wrap inline elements with anonymous boxes
    fn collapse_inline(&mut self, node_id: NodeId) {
        let Some(node) = self.nodes.get(&node_id) else {
            eprintln!("Node not found: {node_id}");
            return;
        };

        let mut inline_wrapper = None;

        for child_id in node.children.clone() {
            let Some(child) = self.nodes.get_mut(&child_id) else {
                eprintln!("Child not found: {child_id}");
                continue;
            };

            if child.is_inline() {
                if let Some(wrapper_id) = inline_wrapper {
                    let old_parent = child.parent;
                    child.parent = Some(wrapper_id);

                    let Some(wrapper) = self.nodes.get_mut(&wrapper_id) else {
                        eprintln!("Wrapper not found: {wrapper_id}");
                        continue;
                    };

                    wrapper.children.push(child_id);

                    if let Some(old_parent) = old_parent {
                        let Some(old_parent) = self.nodes.get_mut(&old_parent) else {
                            eprintln!("Old parent not found: {old_parent}");
                            continue;
                        };

                        old_parent.children.retain(|id| *id != child_id);
                    }
                } else {
                    let wrapper_node = RenderTreeNode {
                        id: self.next_id,
                        properties: C::PropertyMap::default(),
                        children: vec![child_id],
                        parent: Some(node_id),
                        name: "#anonymous".to_string(),
                        namespace: None,
                        data: RenderNodeData::AnonymousInline,
                        cache: L::Cache::default(),
                        layout: L::Layout::default(),
                    };
                    let id = wrapper_node.id;

                    self.next_id = self.next_id.next();

                    let old_parent = child.parent;
                    child.parent = Some(id);
                    inline_wrapper = Some(id);

                    self.nodes.insert(id, wrapper_node);

                    if let Some(old_parent) = old_parent {
                        let Some(old_parent) = self.nodes.get_mut(&old_parent) else {
                            eprintln!("Old parent not found: {old_parent}");
                            continue;
                        };

                        if let Some(pos) = old_parent.children.iter().position(|id| *id == child_id) {
                            old_parent.children[pos] = id;
                        }
                    }
                }
            } else {
                inline_wrapper = None;
            }

            self.collapse_inline(child_id)
        }
    }

    pub fn print_tree(&self) {
        self.print_tree_from(self.root, 0);
    }

    fn print_tree_from(&self, node_id: NodeId, depth: usize) {
        let Some(node) = self.nodes.get(&node_id) else {
            return;
        };
        let indent = "  ".repeat(depth);

        let size = node.layout.size();
        let w = size.width;
        let h = size.height;

        let pos = node.layout.rel_pos();
        let x = pos.x;
        let y = pos.y;

        println!("{indent}{node_id}: {} @ ({x}:{y}) [{w}x{h}]", node.name);

        for child_id in &node.children {
            self.print_tree_from(*child_id, depth + 1);
        }
    }
}

impl<L: Layouter, C: CssSystem> gosub_shared::traits::render_tree::RenderTree<C> for RenderTree<L, C> {
    type NodeId = NodeId;
    type Node = RenderTreeNode<L, C>;

    fn root(&self) -> Self::NodeId {
        self.root
    }

    fn get_node(&self, id: Self::NodeId) -> Option<&Self::Node> {
        self.get_node(id)
    }

    fn get_node_mut(&mut self, id: Self::NodeId) -> Option<&mut Self::Node> {
        self.get_node_mut(id)
    }

    fn get_children(&self, id: Self::NodeId) -> Option<Vec<Self::NodeId>> {
        self.get_children(id).cloned()
    }
}

impl<L: Layouter, C: CssSystem> gosub_shared::traits::render_tree::RenderTreeNode<C> for RenderTreeNode<L, C> {
    fn props(&self) -> &C::PropertyMap {
        &self.properties
    }

    fn props_mut(&mut self) -> &mut C::PropertyMap {
        &mut self.properties
    }
}

// Generates a declaration property and adds it to the css_map_entry

pub enum RenderNodeData<L: Layouter> {
    Document,
    Element { attributes: HashMap<String, String> },
    Text(Box<TextData<L>>),
    AnonymousInline,
}

impl<L: Layouter> Debug for RenderNodeData<L> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            RenderNodeData::Document => f.write_str("Document"),
            RenderNodeData::Element { attributes } => {
                f.debug_struct("Element").field("attributes", attributes).finish()
            }
            RenderNodeData::Text(data) => f.debug_struct("TextData").field("data", data).finish(),
            RenderNodeData::AnonymousInline => f.write_str("AnonymousInline"),
        }
    }
}

// impl<L: Layouter> Debug for RenderNodeData<L> {
//     fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
//         match self {
//             RenderNodeData::Document => f.write_str("Document"),
//             RenderNodeData::Element => {
//                 f.debug_struct("ElementData").field("data", data).finish()
//             }
//             RenderNodeData::Text(data) => f.debug_struct("TextData").field("data", data).finish(),
//             RenderNodeData::AnonymousInline => f.write_str("AnonymousInline"),
//         }
//     }
// }
//

pub struct TextData<L: Layouter> {
    pub text: String,
    pub layout: Option<L::TextLayout>,
}

impl<L: Layouter> Debug for TextData<L> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextData")
            .field("text", &self.text)
            .field("layout", &self.layout.as_ref().map(|x| x.dbg_layout()))
            .finish()
    }
}

pub enum ControlFlow<T> {
    Ok(T),
    Drop,
    Error(anyhow::Error),
}

impl<L: Layouter> RenderNodeData<L> {
    pub fn from_node_data<N: DocumentNode<C>, C: CssSystem>(node: NodeData<C, N>) -> ControlFlow<Self> {
        ControlFlow::Ok(match node {
            NodeData::Element(d) => RenderNodeData::Element {
                attributes: d.attributes().clone(),
            },
            NodeData::Text(data) => {
                let text = pre_transform_text(data.string_value());

                RenderNodeData::Text(Box::new(TextData { text, layout: None }))
            }
            NodeData::Document(_) => RenderNodeData::Document,
            _ => return ControlFlow::Drop,
        })
    }
}

fn pre_transform_text(text: String) -> String {
    let mut new_text = String::with_capacity(text.len());

    let mut last_was_ws = false;
    for c in text.chars() {
        if c.is_whitespace() {
            if !last_was_ws {
                new_text.push(' ');
                last_was_ws = true;
            }
        } else {
            new_text.push(c);
            last_was_ws = false;
        }
    }

    new_text
}

pub struct RenderTreeNode<L: Layouter, C: CssSystem> {
    pub id: NodeId,
    pub properties: C::PropertyMap,
    pub children: Vec<NodeId>,
    pub parent: Option<NodeId>,
    pub name: String,
    pub namespace: Option<String>,
    pub data: RenderNodeData<L>,
    pub cache: L::Cache,
    pub layout: L::Layout,
}

impl<L: Layouter, C: CssSystem> Debug for RenderTreeNode<L, C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderTreeNode")
            .field("id", &self.id)
            .field("properties", &self.properties)
            .field("children", &self.children)
            .field("parent", &self.parent)
            .field("name", &self.name)
            .field("namespace", &self.namespace)
            .field("data", &self.data)
            .finish()
    }
}

impl<L: Layouter, C: CssSystem> RenderTreeNode<L, C> {
    /// Returns true if the node is an element node
    pub fn is_element(&self) -> bool {
        matches!(self.data, RenderNodeData::Element { .. })
    }

    /// Returns true if the node is a text node
    pub fn is_text(&self) -> bool {
        matches!(self.data, RenderNodeData::Text(_))
    }

    /// Returns the requested property for the node
    pub fn get_property(&mut self, prop_name: &str) -> Option<&mut C::Property> {
        self.properties.get_mut(prop_name)
    }

    pub fn is_inline(&mut self) -> bool {
        if matches!(self.data, RenderNodeData::Text(_)) {
            return true;
        }

        if let Some(d) = self.properties.get("display").and_then(|prop| {
            let val = prop.as_string()?;

            // const NON_INLINE_DISPLAYS: [&str; 6] = ["block", "flex", "grid", "table", "list-item", "none"];
            // if NON_INLINE_DISPLAYS.contains(&val.as_str()) {
            //     return Some(false);
            // } //TODO: somehow this causes problems with the inline elements

            if val == "inline" || val == "inline-block" || val == "inline-table" || val == "inline-flex" {
                return Some(true);
            }

            None
        }) {
            return d;
        }

        let tag_name = self.name.to_lowercase();

        INLINE_ELEMENTS.contains(&tag_name.as_str())
    }
}

impl<L: Layouter, C: CssSystem> HasTextLayout<L> for RenderTreeNode<L, C> {
    fn set_text_layout(&mut self, layout: L::TextLayout) {
        if let RenderNodeData::Text(text) = &mut self.data {
            text.layout = Some(layout);
        }
    }
}

impl<L: Layouter, C: CssSystem> layout::Node for RenderTreeNode<L, C> {
    type Property = C::Property;

    fn get_property(&self, name: &str) -> Option<&Self::Property> {
        self.properties.get(name)
    }
    fn text_data(&self) -> Option<&str> {
        if let RenderNodeData::Text(text) = &self.data {
            Some(&text.text)
        } else {
            None
        }
    }

    fn text_size(&self) -> Option<Size> {
        if let RenderNodeData::Text(text) = &self.data {
            text.layout.as_ref().map(|layout| layout.size())
        } else {
            None
        }
    }

    fn is_anon_inline_parent(&self) -> bool {
        matches!(self.data, RenderNodeData::AnonymousInline)
    }
}

/// Generates a render tree for the given document based on its loaded stylesheets
pub fn generate_render_tree<L: Layouter, D: Document<C>, C: CssSystem>(
    document: DocumentHandle<D, C>,
) -> Result<RenderTree<L, C>> {
    let render_tree = RenderTree::from_document(document);

    Ok(render_tree)
}

// pub fn walk_render_tree(tree: &RenderTree, visitor: &mut Box<dyn TreeVisitor<RenderTreeNode>>) {
//     let root = tree.get_root();
//     internal_walk_render_tree(tree, root, visitor);
// }
//
// fn internal_walk_render_tree(
//     tree: &RenderTree,
//     node: &RenderTreeNode,
//     visitor: &mut Box<dyn TreeVisitor<RenderTreeNode>>,
// ) {
//     // Enter node
//     match &node.data {
//         RenderNodeData::Document(document) => visitor.document_enter(tree, node, document),
//         RenderNodeData::DocType(doctype) => visitor.doctype_enter(tree, node, doctype),
//         RenderNodeData::Text(text) => visitor.text_enter(tree, node, &text.into()),
//         RenderNodeData::Comment(comment) => visitor.comment_enter(tree, node, comment),
//         RenderNodeData::Element(element) => visitor.element_enter(tree, node, element),
//     }
//
//     for child_id in &node.children {
//         if tree.nodes.contains_key(child_id) {
//             let child_node = tree.nodes.get(child_id).expect("node");
//             internal_walk_render_tree(tree, child_node, visitor);
//         }
//     }
//
//     // Leave node
//     match &node.data {
//         RenderNodeData::Document(document) => visitor.document_leave(tree, node, document),
//         RenderNodeData::DocType(doctype) => visitor.doctype_leave(tree, node, doctype),
//         RenderNodeData::Text(text) => visitor.text_leave(tree, node, &text.into()),
//         RenderNodeData::Comment(comment) => visitor.comment_leave(tree, node, comment),
//         RenderNodeData::Element(element) => visitor.element_leave(tree, node, element),
//     }
// }
//
// pub trait TreeVisitor<Node> {
//     fn document_enter(&mut self, tree: &RenderTree, node: &RenderTreeNode, data: &DocumentData);
//     fn document_leave(&mut self, tree: &RenderTree, node: &RenderTreeNode, data: &DocumentData);
//
//     fn doctype_enter(&mut self, tree: &RenderTree, node: &RenderTreeNode, data: &DocTypeData);
//     fn doctype_leave(&mut self, tree: &RenderTree, node: &RenderTreeNode, data: &DocTypeData);
//
//     fn text_enter(&mut self, tree: &RenderTree, node: &RenderTreeNode, data: &TextData);
//     fn text_leave(&mut self, tree: &RenderTree, node: &RenderTreeNode, data: &TextData);
//
//     fn comment_enter(&mut self, tree: &RenderTree, node: &RenderTreeNode, data: &CommentData);
//     fn comment_leave(&mut self, tree: &RenderTree, node: &RenderTreeNode, data: &CommentData);
//
//     fn element_enter(&mut self, tree: &RenderTree, node: &RenderTreeNode, data: &ElementData);
//     fn element_leave(&mut self, tree: &RenderTree, node: &RenderTreeNode, data: &ElementData);
// }
