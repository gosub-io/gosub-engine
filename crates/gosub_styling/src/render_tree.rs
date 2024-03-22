use std::collections::HashMap;

use gosub_html5::node::data::comment::CommentData;
use gosub_html5::node::data::doctype::DocTypeData;
use gosub_html5::node::data::document::DocumentData;
use gosub_html5::node::data::element::ElementData;
use gosub_html5::node::data::text::TextData;
use gosub_html5::node::{NodeData, NodeId};
use gosub_html5::parser::document::{DocumentHandle, TreeIterator};
use gosub_shared::types::Result;

use crate::css_values::{
    match_selector, CssProperties, CssProperty, CssValue, DeclarationProperty,
};

/// Map of all declared values for all nodes in the document
#[derive(Default)]
pub struct RenderTree {
    nodes: HashMap<NodeId, RenderTreeNode>,
    root: NodeId,
}

impl RenderTree {
    // Generates a new render tree with a root node
    pub fn with_capacity(capacity: usize) -> Self {
        let mut tree = Self {
            nodes: HashMap::with_capacity(capacity),
            root: NodeId::root(),
        };

        tree.insert_node(
            NodeId::root(),
            RenderTreeNode {
                id: NodeId::root(),
                properties: CssProperties::new(),
                children: Vec::new(),
                parent: None,
                name: String::from("root"),
                namespace: None,
                data: NodeData::Document(DocumentData::default()),
            },
        );

        tree
    }

    /// Returns the root node of the render tree
    pub fn get_root(&self) -> &RenderTreeNode {
        self.nodes.get(&self.root).expect("root node")
    }

    /// INserts a new node into the render tree, note that you are responsible for the node id
    /// and the children of the node
    pub fn insert_node(&mut self, id: NodeId, node: RenderTreeNode) {
        self.nodes.insert(id, node);
    }

    /// Deletes the node with the given id from the render tree
    pub fn delete_node(&mut self, id: &NodeId) -> Option<(NodeId, RenderTreeNode)> {
        if self.nodes.contains_key(id) {
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
            // if !node.is_element() {
            //     continue;
            // }

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
            let render_tree_node = RenderTreeNode {
                id: current_node_id,
                properties: css_map_entry,
                children: current_node.children.clone(),
                parent: node.parent,
                name: node.name.clone(), // We might be able to move node into render_tree_node
                namespace: node.namespace.clone(),
                data: node.data.clone(),
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
            if let NodeData::Element(element) = &node.data {
                if removable_elements.contains(&element.name.as_str()) {
                    delete_list.append(&mut self.get_child_node_ids(*id));
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
pub struct RenderTreeNode {
    pub id: NodeId,
    pub properties: CssProperties,
    pub children: Vec<NodeId>,
    pub parent: Option<NodeId>,
    pub name: String,
    pub namespace: Option<String>,
    pub data: NodeData,
}

impl RenderTreeNode {
    /// Returns true if the node is an element node
    pub fn is_element(&self) -> bool {
        matches!(self.data, NodeData::Element(_))
    }

    /// Returns true if the node is a text node
    pub fn is_text(&self) -> bool {
        matches!(self.data, NodeData::Text(_))
    }
}

/// Generates a render tree for the given document based on its loaded stylesheets
pub fn generate_render_tree(document: DocumentHandle) -> Result<RenderTree> {
    let mut render_tree = RenderTree::from_document(document);
    render_tree.remove_unrenderable_nodes();
    Ok(render_tree)
}

pub fn walk_render_tree(tree: &RenderTree, visitor: &mut Box<dyn TreeVisitor<RenderTreeNode>>) {
    let root = tree.get_root();
    internal_walk_render_tree(tree, root, visitor);
}

fn internal_walk_render_tree(
    tree: &RenderTree,
    node: &RenderTreeNode,
    visitor: &mut Box<dyn TreeVisitor<RenderTreeNode>>,
) {
    // Enter node
    match &node.data {
        NodeData::Document(document) => visitor.document_enter(tree, node, document),
        NodeData::DocType(doctype) => visitor.doctype_enter(tree, node, doctype),
        NodeData::Text(text) => visitor.text_enter(tree, node, text),
        NodeData::Comment(comment) => visitor.comment_enter(tree, node, comment),
        NodeData::Element(element) => visitor.element_enter(tree, node, element),
    }

    for child_id in &node.children {
        if tree.nodes.contains_key(child_id) {
            let child_node = tree.nodes.get(child_id).expect("node");
            internal_walk_render_tree(tree, child_node, visitor);
        }
    }

    // Leave node
    match &node.data {
        NodeData::Document(document) => visitor.document_leave(tree, node, document),
        NodeData::DocType(doctype) => visitor.doctype_leave(tree, node, doctype),
        NodeData::Text(text) => visitor.text_leave(tree, node, text),
        NodeData::Comment(comment) => visitor.comment_leave(tree, node, comment),
        NodeData::Element(element) => visitor.element_leave(tree, node, element),
    }
}

pub trait TreeVisitor<Node> {
    fn document_enter(&mut self, tree: &RenderTree, node: &RenderTreeNode, data: &DocumentData);
    fn document_leave(&mut self, tree: &RenderTree, node: &RenderTreeNode, data: &DocumentData);

    fn doctype_enter(&mut self, tree: &RenderTree, node: &RenderTreeNode, data: &DocTypeData);
    fn doctype_leave(&mut self, tree: &RenderTree, node: &RenderTreeNode, data: &DocTypeData);

    fn text_enter(&mut self, tree: &RenderTree, node: &RenderTreeNode, data: &TextData);
    fn text_leave(&mut self, tree: &RenderTree, node: &RenderTreeNode, data: &TextData);

    fn comment_enter(&mut self, tree: &RenderTree, node: &RenderTreeNode, data: &CommentData);
    fn comment_leave(&mut self, tree: &RenderTree, node: &RenderTreeNode, data: &CommentData);

    fn element_enter(&mut self, tree: &RenderTree, node: &RenderTreeNode, data: &ElementData);
    fn element_leave(&mut self, tree: &RenderTree, node: &RenderTreeNode, data: &ElementData);
}
