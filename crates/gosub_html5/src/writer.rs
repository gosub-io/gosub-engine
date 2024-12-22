use crate::node::visitor::Visitor;
use gosub_interface::config::HasDocument;
use gosub_interface::document::Document;
use gosub_interface::document_handle::DocumentHandle;
use gosub_interface::node::ElementDataType;
use gosub_interface::node::{CommentDataType, DocTypeDataType, Node, NodeType, TextDataType};
use gosub_shared::node::NodeId;

// Writer to convert a document to a string
pub struct DocumentWriter {
    /// The buffer to write to
    buffer: String,
    /// Whether to include comments in the output
    comments: bool,
}

impl DocumentWriter {
    pub fn write_from_node<C: HasDocument>(node: NodeId, handle: DocumentHandle<C>) -> String {
        let mut w = Self {
            comments: false,
            buffer: String::new(),
        };

        w.visit_node(node, handle);
        w.buffer
    }

    pub fn visit_node<C: HasDocument>(&mut self, id: NodeId, handle: DocumentHandle<C>) {
        let binding = handle.get();
        let node = match binding.node_by_id(id) {
            Some(node) => node,
            None => return,
        };

        match node.type_of() {
            NodeType::DocumentNode => {
                // self.document_enter(node);
                <DocumentWriter as Visitor<C>>::document_enter(self, node);

                self.visit_children(node.children(), handle.clone());
                <DocumentWriter as Visitor<C>>::document_leave(self, node);
            }
            NodeType::DocTypeNode => {
                <DocumentWriter as Visitor<C>>::doctype_enter(self, node);
                self.visit_children(node.children(), handle.clone());
                <DocumentWriter as Visitor<C>>::doctype_leave(self, node);
            }
            NodeType::TextNode => {
                <DocumentWriter as Visitor<C>>::text_enter(self, node);
                self.visit_children(node.children(), handle.clone());
                <DocumentWriter as Visitor<C>>::text_leave(self, node);
            }
            NodeType::CommentNode => {
                <DocumentWriter as Visitor<C>>::comment_enter(self, node);
                self.visit_children(node.children(), handle.clone());
                <DocumentWriter as Visitor<C>>::comment_leave(self, node);
            }
            NodeType::ElementNode => {
                <DocumentWriter as Visitor<C>>::element_enter(self, node);
                self.visit_children(node.children(), handle.clone());
                <DocumentWriter as Visitor<C>>::element_leave(self, node);
            }
        }
    }

    pub fn visit_children<C: HasDocument>(&mut self, children: &[NodeId], handle: DocumentHandle<C>) {
        for child in children {
            self.visit_node(*child, handle.clone());
        }
    }
}

impl<C: HasDocument> Visitor<C> for DocumentWriter {
    fn document_enter(&mut self, _node: &C::Node) {}

    fn document_leave(&mut self, _node: &C::Node) {}

    fn doctype_enter(&mut self, node: &C::Node) {
        if let Some(data) = node.get_doctype_data() {
            self.buffer.push_str("<!DOCTYPE ");
            self.buffer.push_str(data.name());
            self.buffer.push('>');
        }
    }

    fn doctype_leave(&mut self, _node: &C::Node) {}

    fn text_enter(&mut self, node: &C::Node) {
        if let Some(data) = node.get_text_data() {
            self.buffer.push_str(data.value());
        }
    }

    fn text_leave(&mut self, _node: &C::Node) {}

    fn comment_enter(&mut self, node: &C::Node) {
        if let Some(data) = node.get_comment_data() {
            self.buffer.push_str("<!--");
            self.buffer.push_str(data.value());
            self.buffer.push_str("-->");
        }
    }

    fn comment_leave(&mut self, _node: &C::Node) {}

    fn element_enter(&mut self, node: &C::Node) {
        if let Some(data) = node.get_element_data() {
            self.buffer.push('<');
            self.buffer.push_str(data.name());

            for (name, value) in data.attributes() {
                self.buffer.push(' ');
                self.buffer.push_str(name);
                self.buffer.push_str("=\"");
                self.buffer.push_str(value);
                self.buffer.push('"');
            }

            self.buffer.push('>');
        }
    }

    fn element_leave(&mut self, node: &C::Node) {
        if let Some(data) = node.get_element_data() {
            self.buffer.push_str("</");
            self.buffer.push_str(data.name().to_string().as_str());
            self.buffer.push('>');
        }
    }
}
