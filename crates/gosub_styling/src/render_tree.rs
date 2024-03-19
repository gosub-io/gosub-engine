use std::collections::HashMap;

use gosub_html5::node::{NodeData, NodeId};
use gosub_html5::parser::document::{DocumentHandle, TreeIterator};
use gosub_shared::types::Result;

use crate::css_values::{
    match_selector, CssProperties, CssProperty, CssValue, DeclarationProperty,
};

/// Map of all declared values for all nodes in the document
#[derive(Default)]
pub struct RenderTree {
    pub nodes: HashMap<NodeId, RenderTreeNode>,
}

impl RenderTree {
    pub fn delete_node(&mut self, id: &NodeId) -> Option<(NodeId, RenderTreeNode)> {
        self.nodes.remove_entry(id)
    }

    fn remove_unrenderable_nodes(&mut self) {
        // There are more elements that are not renderable, but for now we only remove the most common ones
        let removable_elements = ["head", "script", "style", "svg"];

        let mut delete = Vec::new();

        for (id, node) in &self.nodes {
            if let NodeData::Element(element) = &node.data {
                if removable_elements.contains(&element.name.as_str()) {
                    delete.push(*id);
                    continue;
                }
            }

            // Check CSS styles and remove if not renderable
            if let Some(mut prop) = self.get_property(*id, "display") {
                if prop.compute_value().to_string() == "none" {
                    delete.push(*id);
                    continue;
                }
            }
        }

        for id in delete {
            self.delete_node(&id);
        }
    }
}

pub struct RenderTreeNode {
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

impl RenderTree {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            nodes: HashMap::with_capacity(capacity),
        }
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
}

/// Generates a render tree for the given document based on its loaded stylesheets
pub fn generate_render_tree(document: DocumentHandle) -> Result<RenderTree> {
    // Restart css map
    let mut render_tree = RenderTree::with_capacity(document.get().count_nodes());

    // Iterate the complete document tree
    let tree_iterator = TreeIterator::new(&document);
    for current_node_id in tree_iterator {
        let mut css_map_entry = CssProperties::new();

        let binding = document.get();
        let node = binding
            .get_node_by_id(current_node_id)
            .expect("node not found");
        if !node.is_element() {
            continue;
        }

        for sheet in document.get().stylesheets.iter() {
            for rule in sheet.rules.iter() {
                for selector in rule.selectors().iter() {
                    if !match_selector(DocumentHandle::clone(&document), current_node_id, selector)
                    {
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

        let render_tree_node = RenderTreeNode {
            properties: css_map_entry,
            children: Vec::new(),
            parent: node.parent,
            name: node.name.clone(), // We might be able to move node into render_tree_node
            namespace: node.namespace.clone(),
            data: node.data.clone(),
        };

        render_tree.nodes.insert(current_node_id, render_tree_node);
    }

    for (node_id, render_node) in render_tree.nodes.iter() {
        println!("Node: {:?}", node_id);
        for (prop, values) in render_node.properties.properties.iter() {
            println!("  {}", prop);
            if prop == "color" {
                for decl in values.declared.iter() {
                    println!(
                        "    {:?} {:?} {:?} {:?}",
                        decl.origin, decl.location, decl.value, decl.specificity
                    );
                }
            }
        }
    }

    render_tree.remove_unrenderable_nodes();

    Ok(render_tree)
}
