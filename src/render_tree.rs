use std::borrow::BorrowMut;
use std::{cell::RefCell, rc::Rc};

use crate::html5::node::NodeData;
use crate::html5::parser::document;
use crate::html5::parser::document::{Document, DocumentHandle};
use crate::render_tree::{properties::Rectangle, text::TextNode};

pub mod properties;
pub mod text;
pub mod util;

/// The position of the render cursor used to determine where
/// to draw an object
#[derive(Debug, PartialEq)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

impl Position {
    pub fn new() -> Self {
        Self { x: 0., y: 0. }
    }

    pub fn new_from_existing(position: &Position) -> Self {
        Self {
            x: position.x,
            y: position.y,
        }
    }

    /// Move position to (x, y)
    pub fn move_to(&mut self, x: f64, y: f64) {
        self.x = x;
        self.y = y;
    }

    /// Move position relative to another position.
    /// x = relative.x + x_offset
    /// y = relative.y + y_offset
    pub fn move_relative_to(&mut self, relative_position: &Position, x_offset: f64, y_offset: f64) {
        self.x = relative_position.x + x_offset;
        self.y = relative_position.y + y_offset;
    }

    /// Adjust y by an offset.
    /// y += offset_y
    pub fn offset_y(&mut self, offset_y: f64) {
        self.y += offset_y;
    }

    /// Adjust x by an offset.
    /// x += offset_x
    pub fn offset_x(&mut self, offset_x: f64) {
        self.x += offset_x;
    }
}

impl Default for Position {
    fn default() -> Self {
        Self::new()
    }
}

/// A RenderTree is a data structure to be consumed by a user agent
/// that combines the DOM and CSSOM to compute layouts and styles
/// for objects to draw on the screen.
pub struct RenderTree {
    /// Pointer to the underlying document that builds this render tree
    // TODO: Add CSSOM here as well when we get to that point
    document: DocumentHandle,
    /// Entry point of the render tree
    // TODO: make this a NodeHandle to make operations easier and not
    // have to keep doing borrow(), borrow_mut() etc.
    pub root: Rc<RefCell<Node>>,
    /// Global render cursor position managed by the tree
    /// that determines where to draw objects on the screen.
    /// Whenever a node is created, the current value of the
    /// position is copied into the node.
    pub position: Position,
}

impl RenderTree {
    pub fn new(document: &DocumentHandle) -> Self {
        Self {
            document: Document::clone(document),
            root: Rc::new(RefCell::new(Node::new())),
            position: Position::new(),
        }
    }

    pub fn build(&mut self) {
        // start with a clean root (if build is called multiple times)
        self.root = Rc::new(RefCell::new(Node::new()));

        let tree_iterator = document::TreeIterator::new(&self.document);
        let mut reference_element = Rc::clone(&self.root);

        for current_node_id in tree_iterator {
            let doc_read = self.document.get();
            if let Some(current_node) = doc_read.get_node_by_id(current_node_id) {
                match &current_node.data {
                    NodeData::Element(element) => {
                        let new_node = match element.name.as_str() {
                            "h1" => Node::new_heading1,
                            "h2" => Node::new_heading2,
                            "h3" => Node::new_heading3,
                            "h4" => Node::new_heading4,
                            "h5" => Node::new_heading5,
                            "h6" => Node::new_heading6,
                            "p" => Node::new_paragraph,
                            _ => continue, /* add more here, for now ignore */
                        }(&mut self.position);

                        let new_node = Rc::new(RefCell::new(new_node));

                        util::add_text_node(&reference_element, &new_node);
                        reference_element = Rc::clone(&new_node);
                    }
                    NodeData::Text(text) => {
                        let mut mut_element_ref = reference_element.as_ref().borrow_mut();
                        if let NodeType::Text(element_text) = &mut mut_element_ref.node_type {
                            element_text.value.push_str(text.value());
                        }
                    }
                    _ => { /* ignore */ }
                }
            }
        }
    }
}

/// An individual node that sits inside a RenderTree.
/// A RenderTree Node mimics a Node from the DOM but
/// contains more visual information such as width,
/// height, font sizes, colors, etc. A RenderTree Node
/// can have children just like regular DOM nodes.
#[derive(Debug, PartialEq)]
pub struct Node {
    pub node_type: NodeType,
    // TODO: set default margin/padding for all headings/paragraph
    // items. Maybe we need a Node::new_heading1() which is a wrapper
    // around Text::new_heading1() to set the additional information
    pub margin: Rectangle,
    pub padding: Rectangle,
    // TODO: border and other common properties
    pub parent: Option<Rc<RefCell<Node>>>,
    pub next_sibling: Option<Rc<RefCell<Node>>>,
    pub children: Vec<Rc<RefCell<Node>>>,
    pub position: Position,
}

impl Node {
    pub fn new() -> Self {
        Self {
            node_type: NodeType::Root,
            margin: Rectangle::new(),
            padding: Rectangle::new(),
            parent: None,
            next_sibling: None,
            children: Vec::new(),
            position: Position::new(),
        }
    }

    fn new_text(node: TextNode, margin: f64, position: &mut Position) -> Self {
        position.offset_y(margin);
        let fs = node.font_size;
        let new_node = Node {
            node_type: NodeType::Text(node),
            margin: Rectangle::with_values(margin, 0., 0., margin),
            padding: Rectangle::new(),
            parent: None,
            next_sibling: None,
            children: Vec::new(),
            position: Position::new_from_existing(position),
        };

        position.offset_y(fs + margin);

        new_node
    }

    // I took the margins/font sizes from Chrome dev tools.
    // There are still some slight differences but it's very close

    pub fn new_heading1(position: &mut Position) -> Self {
        let margin = 10.72;
        let heading = TextNode::new_heading1();

        Node::new_text(heading, margin, position)
    }

    pub fn new_heading2(position: &mut Position) -> Self {
        let margin = 9.96;
        let heading = TextNode::new_heading2();

        Node::new_text(heading, margin, position)
    }

    pub fn new_heading3(position: &mut Position) -> Self {
        let margin = 9.36;
        let heading = TextNode::new_heading3();

        Node::new_text(heading, margin, position)
    }

    pub fn new_heading4(position: &mut Position) -> Self {
        let margin = 10.64;
        let heading = TextNode::new_heading4();

        Node::new_text(heading, margin, position)
    }

    pub fn new_heading5(position: &mut Position) -> Self {
        let margin = 11.089;
        let heading = TextNode::new_heading5();

        Node::new_text(heading, margin, position)
    }

    pub fn new_heading6(position: &mut Position) -> Self {
        let margin = 12.489;
        let heading = TextNode::new_heading6();

        Node::new_text(heading, margin, position)
    }

    pub fn new_paragraph(position: &mut Position) -> Self {
        let margin = 8.;
        let paragraph = TextNode::new_paragraph();

        Node::new_text(paragraph, margin, position)
    }

    pub fn add_child(&mut self, child: &Rc<RefCell<Node>>) {
        if let Some(last_child) = &self.children.last().borrow_mut() {
            last_child.as_ref().borrow_mut().next_sibling = Some(Rc::clone(child));
        }
        self.children.push(Rc::clone(child));
    }
}

impl Default for Node {
    fn default() -> Self {
        Self::new()
    }
}

/// Different types of RenderTree Nodes
#[derive(Debug, PartialEq)]
pub enum NodeType {
    /// Serves no purpose besides being the entry point
    Root,
    /// Represents text to render. Usually created from heading or paragraph elements in the DOM.
    Text(TextNode),
    // TODO: add more types as we build out the RenderTree
}

/// Constructs an iterator for a RenderTree
// NOTE: code taken from Document::TreeIterator and modified
// slightly to fit the render tree, not sure if it's possible/complex
// to do something general for both
pub struct TreeIterator {
    current_node: Option<Rc<RefCell<Node>>>,
    node_stack: Vec<Rc<RefCell<Node>>>,
}

impl TreeIterator {
    pub fn new(render_tree: &RenderTree) -> Self {
        Self {
            current_node: None,
            node_stack: vec![Rc::clone(&render_tree.root)],
        }
    }
}

impl Iterator for TreeIterator {
    type Item = Rc<RefCell<Node>>;

    fn next(&mut self) -> Option<Rc<RefCell<Node>>> {
        self.current_node = self.node_stack.pop();
        if let Some(current_node) = &self.current_node {
            self.current_node.as_ref()?;

            if let Some(node) = &self.current_node {
                if let Some(sibling) = &node.borrow().next_sibling {
                    self.node_stack.push(Rc::clone(sibling));
                }

                if let Some(first_child) = node.borrow().children.first() {
                    self.node_stack.push(Rc::clone(first_child));
                }
            }

            return Some(Rc::clone(current_node));
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use crate::{
        bytes::{CharIterator, Encoding},
        html5::parser::{
            document::{Document, DocumentBuilder},
            Html5Parser,
        },
        render_tree::NodeType,
    };

    use super::{Node, RenderTree, TreeIterator};

    #[test]
    fn next_sibling() {
        let mut root = Node::new();
        let t1 = Rc::new(RefCell::new(Node::new_heading1(&mut root.position)));
        root.add_child(&Rc::clone(&t1));
        assert!(t1.as_ref().borrow().next_sibling.is_none());
        let t2 = Rc::new(RefCell::new(Node::new_heading1(&mut root.position)));
        root.add_child(&Rc::clone(&t2));
        assert!(t1.as_ref().borrow().next_sibling.is_some());
        let t1_ref = t1.as_ref().borrow();
        if let Some(sibling) = &t1_ref.next_sibling {
            assert_eq!(sibling.as_ref(), t2.as_ref());
        } else {
            panic!()
        }
    }

    #[test]
    fn text_nodes() {
        // TODO: create an engine API to simplify this process which would be used by user agents
        let html = "<html>\
                <h1>heading1</h1>\
                <h2>heading2</h2>\
                <h3>heading3</h3>\
                <h4>heading4</h4>\
                <h5>heading5</h5>\
                <h6>heading6</h6>\
                <p>paragraph</p>\
            </html>";
        let mut chars = CharIterator::new();
        chars.read_from_str(html, Some(Encoding::UTF8));
        chars.set_confidence(crate::bytes::Confidence::Certain);
        let doc = DocumentBuilder::new_document();
        let _ = Html5Parser::parse_document(&mut chars, Document::clone(&doc), None);

        let mut render_tree = RenderTree::new(&doc);
        render_tree.build();

        let mut tree_iterator = TreeIterator::new(&render_tree);
        let root = tree_iterator.next().unwrap();
        assert_eq!(root.borrow().node_type, NodeType::Root);

        let h1 = tree_iterator.next().unwrap();
        let h1_ref = h1.borrow();
        let NodeType::Text(h1_node) = &h1_ref.node_type else {
            panic!()
        };

        assert_eq!(h1_node.value, "heading1".to_owned());
        assert_eq!(h1_node.font, "Times New Roman".to_owned());
        assert_eq!(h1_node.font_size, 37.);
        assert!(h1_node.is_bold);

        let h2 = tree_iterator.next().unwrap();
        let h2_ref = h2.borrow();
        let NodeType::Text(h2_node) = &h2_ref.node_type else {
            panic!()
        };

        assert_eq!(h2_node.value, "heading2".to_owned());
        assert_eq!(h2_node.font, "Times New Roman".to_owned());
        assert_eq!(h2_node.font_size, 27.5);
        assert!(h2_node.is_bold);

        let h3 = tree_iterator.next().unwrap();
        let h3_ref = h3.borrow();
        let NodeType::Text(h3_node) = &h3_ref.node_type else {
            panic!()
        };

        assert_eq!(h3_node.value, "heading3".to_owned());
        assert_eq!(h3_node.font, "Times New Roman".to_owned());
        assert_eq!(h3_node.font_size, 21.5);
        assert!(h3_node.is_bold);

        let h4 = tree_iterator.next().unwrap();
        let h4_ref = h4.borrow();
        let NodeType::Text(h4_node) = &h4_ref.node_type else {
            panic!()
        };

        assert_eq!(h4_node.value, "heading4".to_owned());
        assert_eq!(h4_node.font, "Times New Roman".to_owned());
        assert_eq!(h4_node.font_size, 18.5);
        assert!(h4_node.is_bold);

        let h5 = tree_iterator.next().unwrap();
        let h5_ref = h5.borrow();
        let NodeType::Text(h5_node) = &h5_ref.node_type else {
            panic!()
        };

        assert_eq!(h5_node.value, "heading5".to_owned());
        assert_eq!(h5_node.font, "Times New Roman".to_owned());
        assert_eq!(h5_node.font_size, 15.5);
        assert!(h5_node.is_bold);

        let h6 = tree_iterator.next().unwrap();
        let h6_ref = h6.borrow();
        let NodeType::Text(h6_node) = &h6_ref.node_type else {
            panic!()
        };

        assert_eq!(h6_node.value, "heading6".to_owned());
        assert_eq!(h6_node.font, "Times New Roman".to_owned());
        assert_eq!(h6_node.font_size, 12.);
        assert!(h6_node.is_bold);

        let p = tree_iterator.next().unwrap();
        let p_ref = p.borrow();
        let NodeType::Text(p_node) = &p_ref.node_type else {
            panic!()
        };

        assert_eq!(p_node.value, "paragraph".to_owned());
        assert_eq!(p_node.font, "Times New Roman".to_owned());
        assert_eq!(p_node.font_size, 18.5);
        assert!(!p_node.is_bold);
    }
}
