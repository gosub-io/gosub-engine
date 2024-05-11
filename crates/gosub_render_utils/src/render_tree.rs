use std::borrow::BorrowMut;
use std::{cell::RefCell, rc::Rc};

use gosub_html5::node::NodeData;
use gosub_html5::parser::document;
use gosub_html5::parser::document::{Document, DocumentHandle};

use crate::render_tree::properties::Position;
use crate::render_tree::{properties::Rectangle, text::TextNode};

pub mod properties;
pub mod text;
pub mod util;

/// A `RenderTree` is a data structure to be consumed by a user agent
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
    #[must_use]
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

                        util::add_node_to_element(&reference_element, &new_node);
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

/// An individual node that sits inside a `RenderTree`.
/// A `RenderTree` Node mimics a Node from the DOM but
/// contains more visual information such as width,
/// height, font sizes, colors, etc. A `RenderTree` Node
/// can have children just like regular DOM nodes.
#[derive(Debug, PartialEq)]
#[repr(C)]
pub struct Node {
    pub node_type: NodeType,
    pub margin: Rectangle,
    pub padding: Rectangle,
    // TODO: border and other common properties
    pub parent: Option<Rc<RefCell<Node>>>,
    pub next_sibling: Option<Rc<RefCell<Node>>>,
    pub children: Vec<Rc<RefCell<Node>>>,
    pub position: Position,
}

impl Node {
    #[must_use]
    pub fn new() -> Self {
        Self {
            node_type: NodeType::Root(true),
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
        let new_node = Self {
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

        Self::new_text(heading, margin, position)
    }

    pub fn new_heading2(position: &mut Position) -> Self {
        let margin = 9.96;
        let heading = TextNode::new_heading2();

        Self::new_text(heading, margin, position)
    }

    pub fn new_heading3(position: &mut Position) -> Self {
        let margin = 9.36;
        let heading = TextNode::new_heading3();

        Self::new_text(heading, margin, position)
    }

    pub fn new_heading4(position: &mut Position) -> Self {
        let margin = 10.64;
        let heading = TextNode::new_heading4();

        Self::new_text(heading, margin, position)
    }

    pub fn new_heading5(position: &mut Position) -> Self {
        let margin = 11.089;
        let heading = TextNode::new_heading5();

        Self::new_text(heading, margin, position)
    }

    pub fn new_heading6(position: &mut Position) -> Self {
        let margin = 12.489;
        let heading = TextNode::new_heading6();

        Self::new_text(heading, margin, position)
    }

    pub fn new_paragraph(position: &mut Position) -> Self {
        let margin = 8.;
        let paragraph = TextNode::new_paragraph();

        Self::new_text(paragraph, margin, position)
    }

    pub fn add_child(&mut self, child: &Rc<RefCell<Self>>) {
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

/// Different types of `RenderTree` Nodes
// NOTE: tag size must be u32 otherwise it wont work with a C enum (originally I tried u8)
#[derive(Debug, PartialEq)]
#[repr(C, u32)]
pub enum NodeType {
    /// Serves no purpose besides being the entry point
    // NOTE: the bool is a dummy value, otherwise it appears to be ignored when transferring to C API
    Root(bool),
    /// Represents text to render. Usually created from heading or paragraph elements in the DOM.
    Text(TextNode),
    // TODO: add more types as we build out the RenderTree
}

/// Constructs an iterator for a `RenderTree`
pub struct TreeIterator {
    current_node: Option<Rc<RefCell<Node>>>,
    node_stack: Vec<Rc<RefCell<Node>>>,
}

impl TreeIterator {
    #[must_use]
    pub fn new(render_tree: &RenderTree) -> Self {
        Self {
            current_node: None,
            node_stack: vec![Rc::clone(&render_tree.root)],
        }
    }

    #[must_use]
    pub fn current(&self) -> Option<Rc<RefCell<Node>>> {
        if let Some(node) = &self.current_node {
            return Some(Rc::clone(node));
        }

        None
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

    use crate::render_tree::Node;

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
}
