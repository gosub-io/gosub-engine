use std::collections::HashMap;
use gosub_css3::stylesheet::{CssDeclaration, CssSelector, CssStylesheet, CssValue};

use gosub_html5::node::data::comment::CommentData;
use gosub_html5::node::data::doctype::DocTypeData;
use gosub_html5::node::data::document::DocumentData;
use gosub_html5::node::data::element::ElementData;
use gosub_html5::node::data::text::TextData;
use gosub_html5::node::{NodeData, NodeId};
use gosub_html5::parser::document::{DocumentHandle, TreeIterator};
use gosub_shared::types::Result;

use crate::styling::{
    match_selector, CssProperties, CssProperty, DeclarationProperty,
};
use crate::prerender_text::PrerenderText;

/// Map of all declared values for all nodes in the document
#[derive(Default, Debug)]
pub struct RenderTree {
    pub nodes: HashMap<NodeId, RenderTreeNode>,
    pub root: NodeId,
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
                data: RenderNodeData::Document(DocumentData::default()),
            },
        );

        tree
    }

    /// Returns the root node of the render tree
    pub fn get_root(&self) -> &RenderTreeNode {
        self.nodes.get(&self.root).expect("root node")
    }

    /// Returns the node with the given id
    pub fn get_node(&self, id: NodeId) -> Option<&RenderTreeNode> {
        self.nodes.get(&id)
    }

    /// Returns a mutable reference to the node with the given id
    pub fn get_node_mut(&mut self, id: NodeId) -> Option<&mut RenderTreeNode> {
        self.nodes.get_mut(&id)
    }

    /// Returns the children of the given node
    pub fn get_children(&self, id: NodeId) -> Option<&Vec<NodeId>> {
        self.nodes.get(&id).map(|node| &node.children)
    }

    /// Inserts a new node into the render tree, note that you are responsible for the node id
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
                            let property_name = declaration.property.clone();
                            let decl = CssDeclaration {
                                property: property_name.to_string(),
                                values: declaration.values.clone(),
                                important: declaration.important,
                            };
                            self.add_property_to_map(&mut css_map_entry, sheet, selector, &decl);

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
                        )
                        .ok();
                    };
                };

                RenderNodeData::from_node_data(current_node.data.clone(), None).ok()
            };

            let Some(data) = data() else {
                eprintln!("Failed to create node data for node: {:?}", current_node_id);
                continue;
            };

            let render_tree_node = RenderTreeNode {
                id: current_node_id,
                properties: css_map_entry,
                children: current_node.children.clone(),
                parent: node.parent,
                name: node.name.clone(), // We might be able to move node into render_tree_node
                namespace: node.namespace.clone(),
                data,
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

    // Generates a declaration property and adds it to the css_map_entry
    fn add_property_to_map(&self, css_map_entry: &mut CssProperties, sheet: &CssStylesheet, selector: &CssSelector, declaration: &CssDeclaration) {
        let property_name = declaration.property.clone();
        let entry = CssProperty::new(property_name.as_str());

        // If the property is a shorthand css property, we need fetch the individual properties
        // It's possible that need to recurse here as these individual properties can be shorthand as well
        if entry.is_shorthand() {
            for property_name in entry.get_props_from_shorthand() {
                let decl = CssDeclaration {
                    property: property_name.to_string(),
                    values: declaration.values.clone(),
                    important: declaration.important,
                };

                self.add_property_to_map(css_map_entry, sheet, selector, &decl);
            }
        }

        let declaration = DeclarationProperty {
            value: CssValue::List(declaration.values.clone()),
            origin: sheet.origin.clone(),
            important: declaration.important,
            location: sheet.location.clone(),
            specificity: selector.specificity(),
        };

        if let std::collections::hash_map::Entry::Vacant(e) =
            css_map_entry.properties.entry(property_name.clone())
        {
            println!("Adding new property: {:?}", property_name);
            // Generate new property in the css map
            let mut entry = CssProperty::new(property_name.as_str());
            entry.declared.push(declaration);
            e.insert(entry);
        } else {
            println!("Updating on property: {:?}", property_name);

            // Just add the declaration to the existing property
            let entry = css_map_entry.properties.get_mut(&property_name).unwrap();
            entry.declared.push(declaration);
        }
    }
}

#[derive(Debug)]
pub enum RenderNodeData {
    Document(DocumentData),
    Element(Box<ElementData>),
    Text(PrerenderText),
    Comment(CommentData),
    //are these really needed in the render tree?
    DocType(DocTypeData),
}

impl RenderNodeData {
    pub fn from_node_data(node: NodeData, props: Option<&mut CssProperties>) -> Result<Self> {
        Ok(match node {
            NodeData::Document(data) => RenderNodeData::Document(data),
            NodeData::Element(data) => RenderNodeData::Element(data),
            NodeData::Text(data) => {
                let props = props.ok_or(anyhow::anyhow!("No properties found"))?;
                let ff;
                if let Some(prop) = props.get("font-family") {
                    prop.compute_value();
                    ff = if let CssValue::String(ref font_family) = prop.actual {
                        font_family.clone()
                    } else {
                        String::from("Arial")
                    };
                } else {
                    ff = String::from("Arial")
                };

                let ff = ff
                    .trim()
                    .split(',')
                    .map(|ff| ff.to_string())
                    .collect::<Vec<String>>();

                let fs;

                if let Some(prop) = props.get("font-size") {
                    prop.compute_value();

                    fs = if let CssValue::String(ref fs) = prop.actual {
                        if fs.ends_with("px") {
                            fs.trim_end_matches("px").parse::<f32>().unwrap_or(12.0)
                        } else {
                            12.01
                        }
                    } else {
                        12.02
                    };
                } else {
                    fs = 12.03
                };

                let text = PrerenderText::new(data.value.clone(), fs, ff)?;
                RenderNodeData::Text(text)
            }
            NodeData::Comment(data) => RenderNodeData::Comment(data),
            NodeData::DocType(data) => RenderNodeData::DocType(data),
        })
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
    pub data: RenderNodeData,
}

impl RenderTreeNode {
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
        RenderNodeData::Document(document) => visitor.document_enter(tree, node, document),
        RenderNodeData::DocType(doctype) => visitor.doctype_enter(tree, node, doctype),
        RenderNodeData::Text(text) => visitor.text_enter(tree, node, &text.into()),
        RenderNodeData::Comment(comment) => visitor.comment_enter(tree, node, comment),
        RenderNodeData::Element(element) => visitor.element_enter(tree, node, element),
    }

    for child_id in &node.children {
        if tree.nodes.contains_key(child_id) {
            let child_node = tree.nodes.get(child_id).expect("node");
            internal_walk_render_tree(tree, child_node, visitor);
        }
    }

    // Leave node
    match &node.data {
        RenderNodeData::Document(document) => visitor.document_leave(tree, node, document),
        RenderNodeData::DocType(doctype) => visitor.doctype_leave(tree, node, doctype),
        RenderNodeData::Text(text) => visitor.text_leave(tree, node, &text.into()),
        RenderNodeData::Comment(comment) => visitor.comment_leave(tree, node, comment),
        RenderNodeData::Element(element) => visitor.element_leave(tree, node, element),
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


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shorthand_props() {
        let html = r#"
            <!DOCTYPE html>
            <html>
            <head>
                <style>
                    .container {
                        background: red;
                        border: 1px solid black;
                        border-radius: 5px;
                        margin: 10px;
                    }
                </style>
            </head>
            <body>
                <div class="container">
                    <p>Some text</p>
                </div>
            </body>
            </html>
        "#;

        let document = DocumentHandle::from_html(html).unwrap();
        let mut render_tree = generate_render_tree(document).unwrap();


        let render_node = render_tree.get_node_mut(NodeId::from(11)).unwrap();


        // These props should exist
        assert_eq!(render_node.properties.properties.len(), 33);
        assert!(render_node.properties.properties.contains_key("border-radius"));
        assert!(render_node.properties.properties.contains_key("border-width"));
        assert!(render_node.properties.properties.contains_key("border-top-width"));
        assert!(render_node.properties.properties.contains_key("border-bottom-width"));
        assert!(render_node.properties.properties.contains_key("border-left-width"));
        assert!(render_node.properties.properties.contains_key("margin"));
        assert!(render_node.properties.properties.contains_key("border"));
        assert!(render_node.properties.properties.contains_key("background"));
        assert!(render_node.properties.properties.contains_key("background-color"));
        assert!(render_node.properties.properties.contains_key("border-color"));
        assert!(render_node.properties.properties.contains_key("margin-top"));
        assert!(render_node.properties.properties.contains_key("margin-bottom"));
        // This prop should not exist
        assert!(! render_node.properties.properties.contains_key("display"));


        assert_eq!(render_node.get_property("border").unwrap().compute_value(), &CssValue::List(vec![
            CssValue::Unit(1.0, "px".to_string()),
            CssValue::String("solid".to_string()),
            CssValue::String("black".to_string())
        ]));
        assert_eq!(render_node.get_property("border-color").unwrap().compute_value(), &CssValue::String("black".to_string()));
        assert_eq!(render_node.get_property("border-width").unwrap().compute_value(), &CssValue::String("1px".to_string()));
        assert_eq!(render_node.get_property("border-left-width").unwrap().compute_value(), &CssValue::String("1px".to_string()));
        assert_eq!(render_node.get_property("border-top-width").unwrap().compute_value(), &CssValue::String("1px".to_string()));
        assert_eq!(render_node.get_property("border-right-width").unwrap().compute_value(), &CssValue::String("1px".to_string()));
        assert_eq!(render_node.get_property("border-bottom-width").unwrap().compute_value(), &CssValue::String("1px".to_string()));
        assert_eq!(render_node.get_property("border-style").unwrap().compute_value(), &CssValue::String("solid".to_string()));

        dbg!(&render_node.properties.properties);
    }
}