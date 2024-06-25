use crate::{
    node::{Node, NodeData, NodeId},
    parser::document::Document,
    visit::Visitor,
};

impl Document {
    pub fn write_document(&self) -> String {
        Writer::write_from_node(NodeId::root(), self)
    }

    pub fn write_from_node(&self, node: NodeId) -> String {
        Writer::write_from_node(node, self)
    }
}

struct Writer {
    buffer: String,
    comments: bool,
}

impl Writer {
    pub fn write_from_node(node: NodeId, doc: &Document) -> String {
        let mut w = Self {
            comments: false,
            buffer: String::new(),
        };

        w.visit_node(node, doc);

        w.buffer
    }

    pub fn visit_node(&mut self, id: NodeId, doc: &Document) {
        let Some(node) = doc.get_node_by_id(id) else {
            return;
        };

        match node.data {
            NodeData::Document(ref data) => {
                self.document_enter(node, data);

                self.visit_children(&node.children, doc);

                self.document_leave(node, data);
            }

            NodeData::DocType(ref data) => {
                self.doctype_enter(node, data);

                self.visit_children(&node.children, doc);

                self.doctype_leave(node, data);
            }

            NodeData::Text(ref data) => {
                self.text_enter(node, data);

                self.visit_children(&node.children, doc);

                self.text_leave(node, data);
            }
            NodeData::Comment(ref data) => {
                self.comment_enter(node, data);

                self.visit_children(&node.children, doc);

                self.comment_leave(node, data);
            }

            NodeData::Element(ref data) => {
                self.element_enter(node, data);

                self.visit_children(&node.children, doc);

                self.element_leave(node, data);
            }
        }
    }

    pub fn visit_children(&mut self, children: &Vec<NodeId>, doc: &Document) {
        for child in children {
            self.visit_node(*child, doc);
        }
    }
}

impl Visitor<Node> for Writer {
    fn text_enter(&mut self, _node: &Node, data: &crate::node::data::text::TextData) {
        self.buffer.push_str(&data.value);
    }

    fn text_leave(&mut self, _node: &Node, _data: &crate::node::data::text::TextData) {}

    fn doctype_enter(&mut self, _node: &Node, data: &crate::node::data::doctype::DocTypeData) {
        self.buffer.push_str("<!DOCTYPE ");
        self.buffer.push_str(&data.name);
        self.buffer.push('>');
    }

    fn doctype_leave(&mut self, _node: &Node, _data: &crate::node::data::doctype::DocTypeData) {}

    fn comment_enter(&mut self, _node: &Node, data: &crate::node::data::comment::CommentData) {
        if self.comments {
            self.buffer.push_str("<!--");
            self.buffer.push_str(&data.value);
            self.buffer.push_str("-->");
        }
    }

    fn comment_leave(&mut self, _node: &Node, _data: &crate::node::data::comment::CommentData) {}

    fn element_enter(&mut self, _node: &Node, data: &crate::node::data::element::ElementData) {
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

    fn element_leave(&mut self, _node: &Node, data: &crate::node::data::element::ElementData) {
        self.buffer.push_str("</");
        self.buffer.push_str(&data.name);
        self.buffer.push('>');
    }

    fn document_enter(&mut self, _node: &Node, _data: &crate::node::data::document::DocumentData) {}

    fn document_leave(&mut self, _node: &Node, _data: &crate::node::data::document::DocumentData) {}
}
