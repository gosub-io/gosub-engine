//! Style calculator
//!
//! This module calculates the style properties for all nodes in the tree.

use crate::css3::node::Node;
use crate::html5::parser::document::{DocumentHandle, TreeIterator};

pub fn calculate_styles(doc: DocumentHandle, _stylesheets: &[Node])
{
    let tree_iterator = TreeIterator::new(&DocumentHandle::clone(&doc));
    for node_id in tree_iterator {
        let binding = doc.get();
        let node = binding.get_node_by_id(node_id).unwrap();
        match node. {
            Node::Element(element) => {
                // println!("element: {:?}", element)
            }
            Node::Text(text) => {
                println!("text: {:?}", text)
            }
            Node::Comment(comment) => {
                // println!("comment: {:?}", comment)
            }
            Node::Document(document) => {
              //  println!("document: {:?}", document)
            }
            Node::DocumentType(document_type) => {
            //    println!("document_type: {:?}", document_type)
            }
        }
        // println!("node: {:?}", node)
    }
}