use std::collections::HashMap;
use std::fmt::Debug;

use gosub_html5::node::data::element::ElementData;
use gosub_html5::node::{NodeData, NodeId};
use gosub_html5::parser::document::{DocumentHandle, TreeIterator};
use gosub_render_backend::geo::{Size, FP};
use gosub_render_backend::layout::{LayoutTree, Layouter, Node};
use gosub_render_backend::{PreRenderText, RenderBackend};
use gosub_shared::types::Result;
use gosub_typeface::DEFAULT_FS;

use crate::css_values::{
    match_selector, CssProperties, CssProperty, CssValue, DeclarationProperty,
};

/// Map of all declared values for all nodes in the document
#[derive(Debug)]
pub struct RenderTree<B: RenderBackend, L: Layouter> {
    pub nodes: HashMap<NodeId, RenderTreeNode<B, L>>,
    pub root: NodeId,
    pub dirty: bool,
}

#[allow(unused)]
impl<B: RenderBackend, L: Layouter> LayoutTree<L> for RenderTree<B, L> {
    type NodeId = NodeId;

    type Node = RenderTreeNode<B, L>;

    fn children(&self, id: Self::NodeId) -> Option<Vec<Self::NodeId>> {
        self.get_children(id).cloned()
    }

    fn contains(&self, id: &Self::NodeId) -> bool {
        self.nodes.contains_key(id)
    }

    fn child_count(&self, id: Self::NodeId) -> usize {
        self.child_count(id)
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
        self.get_node(id).map(|node| node.css_dirty).unwrap_or(true)
    }

    fn clean_style(&mut self, id: Self::NodeId) {
        if let Some(node) = self.get_node_mut(id) {
            node.css_dirty = false;
        }
    }

    fn get_node(&mut self, id: Self::NodeId) -> Option<&mut Self::Node> {
        self.get_node_mut(id)
    }
}

impl<B: RenderBackend, L: Layouter> RenderTree<B, L> {
    // Generates a new render tree with a root node
    pub fn with_capacity(capacity: usize) -> Self {
        let mut tree = Self {
            nodes: HashMap::with_capacity(capacity),
            root: NodeId::root(),
            dirty: false,
        };

        tree.insert_node(
            NodeId::root(),
            RenderTreeNode {
                id: NodeId::root(),
                properties: CssProperties::new(),
                css_dirty: true,
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
    pub fn get_root(&self) -> &RenderTreeNode<B, L> {
        self.nodes.get(&self.root).expect("root node")
    }

    /// Returns the node with the given id
    pub fn get_node(&self, id: NodeId) -> Option<&RenderTreeNode<B, L>> {
        self.nodes.get(&id)
    }

    /// Returns a mutable reference to the node with the given id
    pub fn get_node_mut(&mut self, id: NodeId) -> Option<&mut RenderTreeNode<B, L>> {
        self.nodes.get_mut(&id)
    }

    /// Returns the children of the given node
    pub fn get_children(&self, id: NodeId) -> Option<&Vec<NodeId>> {
        self.nodes.get(&id).map(|node| &node.children)
    }

    /// Returns the children of the given node
    pub fn child_count(&self, id: NodeId) -> usize {
        self.nodes
            .get(&id)
            .map(|node| node.children.len())
            .unwrap_or(0)
    }

    /// Inserts a new node into the render tree, note that you are responsible for the node id
    /// and the children of the node
    pub fn insert_node(&mut self, id: NodeId, node: RenderTreeNode<B, L>) {
        self.nodes.insert(id, node);
    }

    /// Deletes the node with the given id from the render tree
    pub fn delete_node(&mut self, id: &NodeId) -> Option<(NodeId, RenderTreeNode<B, L>)> {
        println!("Deleting node: {id:?}");

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
            for prop in props.properties.properties.values_mut() {
                prop.mark_dirty();
            }
        }
    }

    /// Mark the given node as clean, so it will not be recalculated
    pub fn mark_clean(&mut self, node_id: NodeId) {
        if let Some(props) = self.nodes.get_mut(&node_id) {
            for prop in props.properties.properties.values_mut() {
                prop.mark_clean();
            }
        }
    }

    /// Retrieves the property for the given node, or None when not found
    pub fn get_property(&self, node_id: NodeId, prop_name: &str) -> Option<CssProperty> {
        let props = self.nodes.get(&node_id);
        props?;

        props
            .expect("props")
            .properties
            .properties
            .get(prop_name)
            .cloned()
    }

    /// Retrieves the value for the given property for the given node, or None when not found
    pub fn get_all_properties(&self, node_id: NodeId) -> Option<&CssProperties> {
        self.nodes.get(&node_id).map(|props| &props.properties)
    }

    /// Generate a render tree from the given document
    pub fn from_document(document: DocumentHandle) -> Self {
        let mut render_tree = RenderTree::with_capacity(document.get().count_nodes());

        render_tree.generate_from(document);
        render_tree.remove_unrenderable_nodes();

        render_tree
    }

    fn generate_from(&mut self, document: DocumentHandle) {
        // Iterate the complete document tree
        let tree_iterator = TreeIterator::new(&document);
        for current_node_id in tree_iterator {
            let mut css_map_entry = CssProperties::new();

            let binding = document.get();
            let node = binding
                .get_node_by_id(current_node_id)
                .expect("node not found");

            for sheet in document.get().stylesheets.iter() {
                for rule in sheet.rules.iter() {
                    for selector in rule.selectors().iter() {
                        if !match_selector(
                            DocumentHandle::clone(&document),
                            current_node_id,
                            selector,
                        ) {
                            continue;
                        }

                        // Selector matched, so we add all declared values to the map
                        for declaration in rule.declarations().iter() {
                            let prop_name = declaration.property.clone();

                            let declaration = DeclarationProperty {
                                value: CssValue::String(declaration.value.clone()), // @TODO: parse the value into the correct CSSValue
                                origin: sheet.origin.clone(),
                                important: declaration.important,
                                location: sheet.location.clone(),
                                specificity: selector.specificity(),
                            };

                            if let std::collections::hash_map::Entry::Vacant(e) =
                                css_map_entry.properties.entry(prop_name.clone())
                            {
                                let mut entry = CssProperty::new(prop_name.as_str());
                                entry.declared.push(declaration);
                                e.insert(entry);
                            } else {
                                let entry = css_map_entry.properties.get_mut(&prop_name).unwrap();
                                entry.declared.push(declaration);
                            }
                        }
                    }
                }
            }

            let binding = document.get();
            let current_node = binding.get_node_by_id(current_node_id).unwrap();

            let mut data = || {
                if let Some(parent_id) = current_node.parent {
                    if let Some(parent) = self.nodes.get_mut(&parent_id) {
                        let parent_props = Some(&mut parent.properties);

                        return RenderNodeData::from_node_data(
                            current_node.data.clone(),
                            parent_props,
                        );
                    };
                };

                RenderNodeData::from_node_data(current_node.data.clone(), None)
            };

            let data = match data() {
                ControlFlow::Ok(data) => data,
                ControlFlow::Drop => continue,
                ControlFlow::Error(e) => {
                    eprintln!("Failed to create node data for node: {current_node_id:?} ({e}");
                    continue;
                }
            };

            let render_tree_node = RenderTreeNode {
                id: current_node_id,
                properties: css_map_entry,
                children: current_node.children.clone(),
                parent: node.parent,
                name: node.name.clone(), // We might be able to move node into render_tree_node
                namespace: node.namespace.clone(),
                data,
                css_dirty: true,
                cache: L::Cache::default(),
                layout: L::Layout::default(),
            };

            self.nodes.insert(current_node_id, render_tree_node);
        }
    }

    /// Removes all unrenderable nodes from the render tree
    fn remove_unrenderable_nodes(&mut self) {
        // There are more elements that are not renderable, but for now we only remove the most common ones
        let removable_elements = ["head", "script", "style", "svg", "noscript"];

        let mut delete_list = Vec::new();

        for (id, node) in &self.nodes {
            if let RenderNodeData::Element(element) = &node.data {
                if removable_elements.contains(&element.name.as_str()) {
                    println!("removing: {:?}({id})", element.name);
                    delete_list.append(&mut self.get_child_node_ids(*id));
                    delete_list.push(*id);
                    continue;
                }
            }

            if let RenderNodeData::Text(t) = &node.data {
                let mut remove = true;
                for c in t.prerender.value().chars() {
                    if !c.is_ascii_whitespace() {
                        remove = false;
                        break;
                    }
                }

                if remove {
                    delete_list.push(*id);
                    continue;
                }
            }

            // Check CSS styles and remove if not renderable
            if let Some(mut prop) = self.get_property(*id, "display") {
                if prop.compute_value().to_string() == "none" {
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
}

#[derive(Debug)]
pub enum RenderNodeData<B: RenderBackend> {
    Document,
    Element(Box<ElementData>),
    Text(Box<TextData<B>>),
}

pub struct TextData<B: RenderBackend> {
    pub prerender: B::PreRenderText,
}

impl<B: RenderBackend> Debug for TextData<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextData")
            .field("text", &self.prerender.value())
            .field("fs", &self.prerender.fs())
            .finish()
    }
}

pub enum ControlFlow<T> {
    Ok(T),
    Drop,
    Error(anyhow::Error),
}

impl<B: RenderBackend> RenderNodeData<B> {
    pub fn from_node_data(node: NodeData, props: Option<&mut CssProperties>) -> ControlFlow<Self> {
        ControlFlow::Ok(match node {
            NodeData::Element(data) => RenderNodeData::Element(data),
            NodeData::Text(data) => {
                let text = data.value.trim();
                let text = text.replace('\n', "");
                let text = text.replace('\r', "");

                let Some(props) = props else {
                    return ControlFlow::Error(anyhow::anyhow!("No properties found"));
                };

                let font = props.get("font-family").and_then(|prop| {
                    prop.compute_value();

                    if let CssValue::String(font_family) = &prop.actual {
                        return Some(
                            font_family
                                .trim()
                                .split(',')
                                .map(|ff| ff.to_string())
                                .collect::<Vec<String>>(),
                        );
                    }

                    None
                });

                let fs = props
                    .get("font-size")
                    .and_then(|prop| {
                        prop.compute_value();
                        if let CssValue::String(fs) = &prop.actual {
                            if fs.ends_with("px") {
                                return fs[..fs.len() - 2].parse::<f32>().ok();
                            }
                        }
                        None
                    })
                    .unwrap_or(DEFAULT_FS) as FP;

                let prerender = PreRenderText::new(text, font, fs);

                let text = TextData { prerender };

                RenderNodeData::Text(Box::new(text))
            }
            NodeData::Document(_) => RenderNodeData::Document,
            _ => return ControlFlow::Drop,
        })
    }
}

pub struct RenderTreeNode<B: RenderBackend, L: Layouter> {
    pub id: NodeId,
    pub properties: CssProperties,
    pub css_dirty: bool,
    pub children: Vec<NodeId>,
    pub parent: Option<NodeId>,
    pub name: String,
    pub namespace: Option<String>,
    pub data: RenderNodeData<B>,
    pub cache: L::Cache,
    pub layout: L::Layout,
}

impl<B: RenderBackend, L: Layouter> Debug for RenderTreeNode<B, L> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderTreeNode")
            .field("id", &self.id)
            .field("properties", &self.properties)
            .field("css_dirty", &self.css_dirty)
            .field("children", &self.children)
            .field("parent", &self.parent)
            .field("name", &self.name)
            .field("namespace", &self.namespace)
            .field("data", &self.data)
            .finish()
    }
}

impl<B: RenderBackend, L: Layouter> RenderTreeNode<B, L> {
    /// Returns true if the node is an element node
    pub fn is_element(&self) -> bool {
        matches!(self.data, RenderNodeData::Element(_))
    }

    /// Returns true if the node is a text node
    pub fn is_text(&self) -> bool {
        matches!(self.data, RenderNodeData::Text(_))
    }

    /// Returns the requested property for the node
    pub fn get_property(&mut self, prop_name: &str) -> Option<&mut CssProperty> {
        self.properties.properties.get_mut(prop_name)
    }

    /// Returns the requested attribute for the node
    pub fn get_attribute(&self, attr_name: &str) -> Option<&String> {
        match &self.data {
            RenderNodeData::Element(element) => element.attributes.get(attr_name),
            _ => None,
        }
    }
}

impl<B: RenderBackend, L: Layouter> Node for RenderTreeNode<B, L> {
    type Property = CssProperty;

    fn get_property(&mut self, name: &str) -> Option<&mut Self::Property> {
        self.properties.properties.get_mut(name)
    }

    fn text_size(&mut self) -> Option<Size> {
        if let RenderNodeData::Text(text) = &mut self.data {
            Some(text.prerender.prerender())
        } else {
            None
        }
    }
}

impl gosub_render_backend::layout::CssProperty for CssProperty {
    fn compute_value(&mut self) {
        self.compute_value();
    }

    fn unit_to_px(&self) -> f32 {
        self.actual.unit_to_px()
    }

    fn as_string(&self) -> Option<&str> {
        if let CssValue::String(str) = &self.actual {
            Some(str)
        } else {
            None
        }
    }

    fn as_percentage(&self) -> Option<f32> {
        if let CssValue::Percentage(percent) = &self.actual {
            Some(*percent)
        } else {
            None
        }
    }

    fn as_unit(&self) -> Option<(f32, &str)> {
        if let CssValue::Unit(value, unit) = &self.actual {
            Some((*value, unit))
        } else {
            None
        }
    }

    fn as_color(&self) -> Option<(f32, f32, f32, f32)> {
        if let CssValue::Color(color) = &self.actual {
            Some((color.r, color.g, color.b, color.a))
        } else {
            None
        }
    }

    fn as_number(&self) -> Option<f32> {
        if let CssValue::Number(num) = &self.actual {
            Some(*num)
        } else {
            None
        }
    }

    fn is_none(&self) -> bool {
        matches!(self.actual, CssValue::None)
    }
}

/// Generates a render tree for the given document based on its loaded stylesheets
pub fn generate_render_tree<B: RenderBackend, L: Layouter>(
    document: DocumentHandle,
) -> Result<RenderTree<B, L>> {
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
