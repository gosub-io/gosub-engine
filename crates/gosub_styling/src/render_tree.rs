use std::collections::HashMap;
use std::fmt::{Debug, Formatter};

use log::warn;

use gosub_css3::stylesheet::{CssDeclaration, CssStylesheet, CssValue, Specificity};
use gosub_html5::node::data::element::ElementData;
use gosub_html5::node::{NodeData, NodeId};
use gosub_html5::parser::document::{Document, DocumentHandle, TreeIterator};
use gosub_render_backend::geo::Size;
use gosub_render_backend::layout::{HasTextLayout, Layout, LayoutTree, Layouter, Node, TextLayout};
use gosub_shared::types::Result;

use crate::functions::{resolve_attr, resolve_calc, resolve_var};
use crate::property_definitions::get_css_definitions;
use crate::shorthands::FixList;
use crate::styling::{
    match_selector, prop_is_inherit, CssProperties, CssProperty, DeclarationProperty,
};

mod desc;

const INLINE_ELEMENTS: [&str; 31] = [
    "a", "abbr", "acronym", "b", "bdo", "big", "br", "button", "cite", "code", "dfn", "em", "i",
    "img", "input", "kbd", "label", "map", "object", "q", "samp", "script", "select", "small",
    "span", "strong", "sub", "sup", "textarea", "tt", "var",
];

/// Map of all declared values for all nodes in the document
#[derive(Debug)]
pub struct RenderTree<L: Layouter> {
    pub nodes: HashMap<NodeId, RenderTreeNode<L>>,
    pub root: NodeId,
    pub dirty: bool,
    next_id: NodeId,
}

#[allow(unused)]
impl<L: Layouter> LayoutTree<L> for RenderTree<L> {
    type NodeId = NodeId;

    type Node = RenderTreeNode<L>;

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

impl<L: Layouter> RenderTree<L> {
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
    pub fn get_root(&self) -> &RenderTreeNode<L> {
        self.nodes.get(&self.root).expect("root node")
    }

    /// Returns the node with the given id
    pub fn get_node(&self, id: NodeId) -> Option<&RenderTreeNode<L>> {
        self.nodes.get(&id)
    }

    /// Returns a mutable reference to the node with the given id
    pub fn get_node_mut(&mut self, id: NodeId) -> Option<&mut RenderTreeNode<L>> {
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
    pub fn insert_node(&mut self, id: NodeId, node: RenderTreeNode<L>) {
        self.nodes.insert(id, node);
    }

    /// Deletes the node with the given id from the render tree
    pub fn delete_node(&mut self, id: &NodeId) -> Option<(NodeId, RenderTreeNode<L>)> {
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

        render_tree
    }

    fn generate_from(&mut self, mut document: DocumentHandle) {
        // Iterate the complete document tree
        let tree_iterator = TreeIterator::new(&document);

        {
            let doc = document.get();

            println!("Stylesheets: {:?}", doc.stylesheets.len());

            for stylesheet in &doc.stylesheets {
                println!("   {:?}", stylesheet.location);
                println!("   {:?}", stylesheet.origin);
            }
        }

        for current_node_id in tree_iterator {
            let mut css_map_entry = CssProperties::new();

            let doc = document.get();
            let node = doc.get_node_by_id(current_node_id).expect("node not found");

            if node_is_undernderable(node) {
                if let Some(parent) = node.parent {
                    if let Some(parent) = self.get_node_mut(parent) {
                        parent.children.retain(|id| *id != current_node_id);
                        // continue
                        //TODO: somehow we still can't continue here, I don't know why
                    }
                }

                drop(doc);

                let mut doc = document.get_mut();

                doc.detach_node_from_parent(current_node_id);

                continue;
            }

            let definitions = get_css_definitions();

            let mut fix_list = FixList::new();

            for sheet in document.get().stylesheets.iter() {
                for rule in sheet.rules.iter() {
                    for selector in rule.selectors().iter() {
                        let (matched, specificity) = match_selector(
                            DocumentHandle::clone(&document),
                            current_node_id,
                            selector,
                        );

                        if !matched {
                            continue;
                        }

                        // Selector matched, so we add all declared values to the map
                        for declaration in rule.declarations().iter() {
                            // Step 1: find the property in our CSS definition list
                            let Some(definition) = definitions.find_property(&declaration.property)
                            else {
                                // If not found, we skip this declaration
                                warn!(
                                    "Definition is not found for property {:?}",
                                    declaration.property
                                );
                                continue;
                            };

                            let value = resolve_functions(&declaration.value, node, &doc);

                            // Check if the declaration matches the definition and return the "expanded" order
                            let res = definition.matches_and_shorthands(&value, &mut fix_list);
                            if !res {
                                warn!("Declaration does not match definition: {:?}", declaration);
                                continue;
                            }

                            // create property for the given values
                            let property_name = declaration.property.clone();
                            let decl = CssDeclaration {
                                property: property_name.to_string(),
                                value,
                                important: declaration.important,
                            };

                            add_property_to_map(
                                &mut css_map_entry,
                                sheet,
                                specificity.clone(),
                                &decl,
                            );
                        }
                    }
                }
            }

            fix_list.resolve_nested(definitions);

            fix_list.apply(&mut css_map_entry);

            let binding = document.get();
            let current_node = binding.get_node_by_id(current_node_id).unwrap();

            let data = || RenderNodeData::from_node_data(current_node.data.clone());

            let data = match data() {
                ControlFlow::Ok(data) => data,
                ControlFlow::Drop => continue,
                ControlFlow::Error(e) => {
                    log::error!("Failed to create node data for node: {current_node_id:?} ({e}");
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

        self.next_id = document.get().peek_next_id();

        self.remove_unrenderable_nodes();

        self.resolve_inheritance(self.root, &Vec::new());

        if L::COLLAPSE_INLINE {
            self.collapse_inline(self.root);
        }

        // self.print_tree();
    }

    fn resolve_inheritance(&mut self, node_id: NodeId, inherit_props: &Vec<(String, CssValue)>) {
        let Some(current_node) = self.get_node(node_id) else {
            return;
        };

        for prop in inherit_props {
            current_node
                .properties
                .properties
                .entry(prop.0.clone())
                .or_insert_with(|| {
                    let mut p = CssProperty::new(prop.0.as_str());

                    p.inherited = prop.1.clone();

                    p
                });
        }

        let mut inherit_props = inherit_props.clone();

        'props: for (name, prop) in &mut current_node.properties.properties {
            prop.compute_value();

            let value = prop.actual.clone();

            if prop_is_inherit(name) {
                for (k, v) in &mut inherit_props {
                    if k == name {
                        *v = value;
                        continue 'props;
                    }
                }

                inherit_props.push((name.clone(), value));
            }
        }

        let Some(children) = self.get_children(node_id) else {
            return;
        };

        for child in children.clone() {
            self.resolve_inheritance(child, &inherit_props);
        }
    }

    /// Removes all unrenderable nodes from the render tree
    fn remove_unrenderable_nodes(&mut self) {
        // There are more elements that are not renderable, but for now we only remove the most common ones
        let mut delete_list = Vec::new();

        for id in self.nodes.keys() {
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
                        properties: CssProperties::new(),
                        css_dirty: true,
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

                        if let Some(pos) = old_parent.children.iter().position(|id| *id == child_id)
                        {
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

// Generates a declaration property and adds it to the css_map_entry
pub fn add_property_to_map(
    css_map_entry: &mut CssProperties,
    sheet: &CssStylesheet,
    specificity: Specificity,
    declaration: &CssDeclaration,
) {
    let property_name = declaration.property.clone();
    // let entry = CssProperty::new(property_name.as_str());

    // If the property is a shorthand css property, we need fetch the individual properties
    // It's possible that need to recurse here as these individual properties can be shorthand as well
    // if entry.is_shorthand() {
    //     for property_name in entry.get_props_from_shorthand() {
    //         let decl = CssDeclaration {
    //             property: property_name.to_string(),
    //             value: declaration.value.clone(),
    //             important: declaration.important,
    //         };
    //
    //         add_property_to_map(css_map_entry, sheet, selector, &decl);
    //     }
    // }
    //
    let declaration = DeclarationProperty {
        // @todo: this seems wrong. We only get the first values from the declared values
        value: declaration.value.first().unwrap().clone(),
        origin: sheet.origin.clone(),
        important: declaration.important,
        location: sheet.location.clone(),
        specificity,
    };

    if let std::collections::hash_map::Entry::Vacant(e) =
        css_map_entry.properties.entry(property_name.clone())
    {
        // Generate new property in the css map
        let mut entry = CssProperty::new(property_name.as_str());
        entry.declared.push(declaration);
        e.insert(entry);
    } else {
        // Just add the declaration to the existing property
        let entry = css_map_entry.properties.get_mut(&property_name).unwrap();
        entry.declared.push(declaration);
    }
}

pub enum RenderNodeData<L: Layouter> {
    Document,
    Element(Box<ElementData>),
    Text(Box<TextData<L>>),
    AnonymousInline,
}

impl<L: Layouter> Debug for RenderNodeData<L> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            RenderNodeData::Document => f.write_str("Document"),
            RenderNodeData::Element(data) => {
                f.debug_struct("ElementData").field("data", data).finish()
            }
            RenderNodeData::Text(data) => f.debug_struct("TextData").field("data", data).finish(),
            RenderNodeData::AnonymousInline => f.write_str("AnonymousInline"),
        }
    }
}

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
    pub fn from_node_data(node: NodeData) -> ControlFlow<Self> {
        ControlFlow::Ok(match node {
            NodeData::Element(data) => RenderNodeData::Element(data),
            NodeData::Text(data) => {
                let text = pre_transform_text(data.value);

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

pub struct RenderTreeNode<L: Layouter> {
    pub id: NodeId,
    pub properties: CssProperties,
    pub css_dirty: bool,
    pub children: Vec<NodeId>,
    pub parent: Option<NodeId>,
    pub name: String,
    pub namespace: Option<String>,
    pub data: RenderNodeData<L>,
    pub cache: L::Cache,
    pub layout: L::Layout,
}

impl<L: Layouter> Debug for RenderTreeNode<L> {
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

impl<L: Layouter> RenderTreeNode<L> {
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

    pub fn is_inline(&mut self) -> bool {
        if matches!(self.data, RenderNodeData::Text(_)) {
            return true;
        }

        if let Some(d) = self.properties.get("display").and_then(|prop| {
            let CssValue::String(val) = &prop.actual else {
                return None;
            };

            // const NON_INLINE_DISPLAYS: [&str; 6] = ["block", "flex", "grid", "table", "list-item", "none"];
            // if NON_INLINE_DISPLAYS.contains(&val.as_str()) {
            //     return Some(false);
            // } //TODO: somehow this causes problems with the inline elements

            if val == "inline"
                || val == "inline-block"
                || val == "inline-table"
                || val == "inline-flex"
            {
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

impl<L: Layouter> HasTextLayout<L> for RenderTreeNode<L> {
    fn set_text_layout(&mut self, layout: L::TextLayout) {
        if let RenderNodeData::Text(text) = &mut self.data {
            text.layout = Some(layout);
        }
    }
}

impl<L: Layouter> Node for RenderTreeNode<L> {
    type Property = CssProperty;

    fn get_property(&mut self, name: &str) -> Option<&mut Self::Property> {
        self.properties.properties.get_mut(name)
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

pub struct Value(pub CssValue);

impl gosub_render_backend::layout::CssProperty for CssProperty {
    type Value = Value;

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

    fn parse_color(&self) -> Option<(f32, f32, f32, f32)> {
        self.actual
            .to_color()
            .map(|color| (color.r, color.g, color.b, color.a))
    }

    fn as_number(&self) -> Option<f32> {
        if let CssValue::Number(num) = &self.actual {
            Some(*num)
        } else {
            None
        }
    }

    fn as_list(&self) -> Option<Vec<Self::Value>> {
        if let CssValue::List(list) = &self.actual {
            Some(list.iter().map(|v| v.clone().into()).collect())
        } else {
            None
        }
    }

    fn is_none(&self) -> bool {
        matches!(self.actual, CssValue::None)
    }
}

impl gosub_render_backend::layout::CssValue for Value {
    fn unit_to_px(&self) -> f32 {
        self.0.unit_to_px()
    }

    fn as_string(&self) -> Option<&str> {
        if let CssValue::String(str) = &self.0 {
            Some(str)
        } else {
            None
        }
    }

    fn as_percentage(&self) -> Option<f32> {
        if let CssValue::Percentage(percent) = &self.0 {
            Some(*percent)
        } else {
            None
        }
    }

    fn as_unit(&self) -> Option<(f32, &str)> {
        if let CssValue::Unit(value, unit) = &self.0 {
            Some((*value, unit))
        } else {
            None
        }
    }

    fn as_color(&self) -> Option<(f32, f32, f32, f32)> {
        if let CssValue::Color(color) = &self.0 {
            Some((color.r, color.g, color.b, color.a))
        } else {
            None
        }
    }

    fn as_number(&self) -> Option<f32> {
        if let CssValue::Number(num) = &self.0 {
            Some(*num)
        } else {
            None
        }
    }

    fn as_list(&self) -> Option<Vec<Self>> {
        if let CssValue::List(list) = &self.0 {
            Some(list.iter().map(|v| v.clone().into()).collect())
        } else {
            None
        }
    }

    fn is_none(&self) -> bool {
        matches!(self.0, CssValue::None)
    }
}

impl From<CssValue> for Value {
    fn from(val: CssValue) -> Self {
        Value(val)
    }
}

/// Generates a render tree for the given document based on its loaded stylesheets
pub fn generate_render_tree<L: Layouter>(document: DocumentHandle) -> Result<RenderTree<L>> {
    let render_tree = RenderTree::from_document(document);

    Ok(render_tree)
}

pub fn node_is_undernderable(node: &gosub_html5::node::Node) -> bool {
    // There are more elements that are not renderable, but for now we only remove the most common ones

    const REMOVABLE_ELEMENTS: [&str; 6] = ["head", "script", "style", "svg", "noscript", "title"];

    if let NodeData::Element(element) = &node.data {
        if REMOVABLE_ELEMENTS.contains(&element.name.as_str()) {
            return true;
        }
    }

    if let NodeData::Text(text) = &node.data {
        if text.value.chars().all(|c| c.is_whitespace()) {
            return true;
        }
    }

    false
}

pub fn resolve_functions(
    value: &[CssValue],
    node: &gosub_html5::node::Node,
    doc: &Document,
) -> Vec<CssValue> {
    let mut result = Vec::with_capacity(value.len()); //TODO: we could give it a &mut Vec and reuse the allocation

    for val in value {
        match val {
            CssValue::Function(func, values) => {
                let resolved = match func.as_str() {
                    "calc" => resolve_calc(values),
                    "attr" => resolve_attr(values, node),
                    "var" => resolve_var(values, doc, node),
                    _ => vec![val.clone()],
                };

                result.extend(resolved);
            }
            _ => result.push(val.clone()),
        }
    }

    result
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
