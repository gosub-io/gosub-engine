use crate::html5_parser::node::{Node, NodeData};
use crate::html5_parser::node_arena::NodeArena;
use crate::html5_parser::parser::quirks::QuirksMode;
use std::fmt;

#[derive(PartialEq, Debug, Copy, Clone)]
pub enum DocumentType {
    HTML,
    IframeSrcDoc,
}

pub struct Document {
    arena: NodeArena,
    pub doctype: DocumentType,   // Document type
    pub quirks_mode: QuirksMode, // Quirks mode
}

impl Default for Document {
    fn default() -> Self {
        Self {
            arena: NodeArena::new(),
            doctype: DocumentType::HTML,
            quirks_mode: QuirksMode::NoQuirks,
        }
    }
}

impl Document {
    // Creates a new document
    pub fn new() -> Self {
        let mut arena = NodeArena::new();
        arena.add_node(Node::new_document());
        Self {
            arena,
            doctype: DocumentType::HTML,
            quirks_mode: QuirksMode::NoQuirks,
        }
    }

    // Fetches a node by id or returns None when no node with this ID is found
    pub fn get_node_by_id(&self, node_id: usize) -> Option<&Node> {
        self.arena.get_node(node_id)
    }

    pub fn get_mut_node_by_id(&mut self, node_id: usize) -> Option<&mut Node> {
        self.arena.get_mut_node(node_id)
    }

    // Add to the document
    pub fn add_node(&mut self, node: Node, parent_id: usize) -> usize {
        let node_id = self.arena.add_node(node);
        self.arena.attach_node(parent_id, node_id);
        node_id
    }

    pub fn append(&mut self, node_id: usize, parent_id: usize) {
        self.arena.attach_node(parent_id, node_id);
    }

    // // append a node to another parent
    // pub fn append(&mut self, node_id: usize, parent_id: usize) {
    //     self.arena.attach_node(parent_id, node_id);
    // }

    // return the root node
    pub fn get_root(&self) -> &Node {
        self.arena.get_node(0).expect("Root node not found !?")
    }
}

impl Document {
    fn display_tree(&self, node: &Node, indent: usize, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut prefix = " ".repeat(indent);
        if !prefix.is_empty() {
            prefix = format!("{}└─ ", prefix);
        }

        match &node.data {
            NodeData::Document => {
                writeln!(f, "{}Document", prefix)?;
            }
            NodeData::Text { value } => {
                writeln!(f, "{}{}", prefix, value)?;
            }
            NodeData::Comment { value } => {
                writeln!(f, "{}<!-- {} -->", prefix, value)?;
            }
            NodeData::Element { name, attributes } => {
                write!(f, "{}<{}", prefix, name)?;
                for (key, value) in attributes.iter() {
                    write!(f, " {}={}", key, value)?;
                }
                writeln!(f, ">")?;
            }
        }

        for child_id in &node.children {
            if let Some(child) = self.arena.get_node(*child_id) {
                self.display_tree(child, indent + 2, f)?;
            }
        }

        Ok(())
    }
}

impl fmt::Display for Document {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.display_tree(self.get_root(), 0, f)
    }
}