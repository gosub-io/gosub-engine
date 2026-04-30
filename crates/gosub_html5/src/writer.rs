use crate::document::document_impl::DocumentImpl;
use crate::node::node_impl::{NodeDataTypeInternal, NodeImpl};
use crate::node::visitor::Visitor;
use gosub_interface::config::HasDocument;
use gosub_interface::node::NodeType;
use gosub_shared::node::NodeId;

pub struct DocumentWriter {
    buffer: String,
    #[allow(dead_code)]
    comments: bool,
}

impl DocumentWriter {
    pub fn write_from_node<C: HasDocument<Document = DocumentImpl<C>>>(node_id: NodeId, doc: &DocumentImpl<C>) -> String {
        let mut w = Self {
            comments: false,
            buffer: String::new(),
        };
        w.visit_node(node_id, doc);
        w.buffer
    }

    fn visit_node<C: HasDocument<Document = DocumentImpl<C>>>(&mut self, id: NodeId, doc: &DocumentImpl<C>) {
        let Some(node) = doc.node_by_id(id) else { return };

        match node.type_of() {
            NodeType::DocumentNode => {
                self.document_enter(node);
                let children: Vec<NodeId> = node.children().to_vec();
                self.visit_children(children, doc);
                self.document_leave(node);
            }
            NodeType::DocTypeNode => {
                self.doctype_enter(node);
                let children: Vec<NodeId> = node.children().to_vec();
                self.visit_children(children, doc);
                self.doctype_leave(node);
            }
            NodeType::TextNode => {
                self.text_enter(node);
                let children: Vec<NodeId> = node.children().to_vec();
                self.visit_children(children, doc);
                self.text_leave(node);
            }
            NodeType::CommentNode => {
                self.comment_enter(node);
                let children: Vec<NodeId> = node.children().to_vec();
                self.visit_children(children, doc);
                self.comment_leave(node);
            }
            NodeType::ElementNode => {
                self.element_enter(node);
                let children: Vec<NodeId> = node.children().to_vec();
                self.visit_children(children, doc);
                self.element_leave(node);
            }
        }
    }

    fn visit_children<C: HasDocument<Document = DocumentImpl<C>>>(&mut self, children: Vec<NodeId>, doc: &DocumentImpl<C>) {
        for child in children {
            self.visit_node(child, doc);
        }
    }
}

impl Visitor for DocumentWriter {
    fn document_enter(&mut self, _node: &NodeImpl) {}
    fn document_leave(&mut self, _node: &NodeImpl) {}

    fn doctype_enter(&mut self, node: &NodeImpl) {
        if let NodeDataTypeInternal::DocType(data) = &node.data {
            self.buffer.push_str("<!DOCTYPE ");
            self.buffer.push_str(&data.name);
            self.buffer.push('>');
        }
    }

    fn doctype_leave(&mut self, _node: &NodeImpl) {}

    fn text_enter(&mut self, node: &NodeImpl) {
        if let NodeDataTypeInternal::Text(data) = &node.data {
            self.buffer.push_str(&data.value);
        }
    }

    fn text_leave(&mut self, _node: &NodeImpl) {}

    fn comment_enter(&mut self, node: &NodeImpl) {
        if let NodeDataTypeInternal::Comment(data) = &node.data {
            self.buffer.push_str("<!--");
            self.buffer.push_str(&data.value);
            self.buffer.push_str("-->");
        }
    }

    fn comment_leave(&mut self, _node: &NodeImpl) {}

    fn element_enter(&mut self, node: &NodeImpl) {
        if let NodeDataTypeInternal::Element(data) = &node.data {
            self.buffer.push('<');
            self.buffer.push_str(&data.name);
            for (name, value) in &data.attributes {
                self.buffer.push(' ');
                self.buffer.push_str(name);
                self.buffer.push_str("=\"");
                self.buffer.push_str(value);
                self.buffer.push('"');
            }
            self.buffer.push('>');
        }
    }

    fn element_leave(&mut self, node: &NodeImpl) {
        if let NodeDataTypeInternal::Element(data) = &node.data {
            self.buffer.push_str("</");
            self.buffer.push_str(&data.name);
            self.buffer.push('>');
        }
    }
}
