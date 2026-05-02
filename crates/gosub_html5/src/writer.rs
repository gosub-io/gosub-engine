use gosub_interface::config::HasDocument;
use gosub_interface::document::Document;
use gosub_interface::node::NodeType;
use gosub_shared::node::NodeId;

pub struct DocumentWriter;

impl DocumentWriter {
    pub fn write_from_node<C: HasDocument>(node_id: NodeId, doc: &C::Document) -> String {
        let mut buffer = String::new();
        write_node::<C>(node_id, doc, &mut buffer);
        buffer
    }
}

fn write_node<C: HasDocument>(id: NodeId, doc: &C::Document, buf: &mut String) {
    match doc.node_type(id) {
        NodeType::DocumentNode => {
            let children: Vec<NodeId> = doc.children(id).to_vec();
            for child in children {
                write_node::<C>(child, doc, buf);
            }
        }
        NodeType::DocTypeNode => {
            if let Some(name) = doc.doctype_name(id) {
                buf.push_str("<!DOCTYPE ");
                buf.push_str(name);
                buf.push('>');
            }
            let children: Vec<NodeId> = doc.children(id).to_vec();
            for child in children {
                write_node::<C>(child, doc, buf);
            }
        }
        NodeType::TextNode => {
            if let Some(value) = doc.text_value(id) {
                buf.push_str(value);
            }
            let children: Vec<NodeId> = doc.children(id).to_vec();
            for child in children {
                write_node::<C>(child, doc, buf);
            }
        }
        NodeType::CommentNode => {
            if let Some(value) = doc.comment_value(id) {
                buf.push_str("<!--");
                buf.push_str(value);
                buf.push_str("-->");
            }
            let children: Vec<NodeId> = doc.children(id).to_vec();
            for child in children {
                write_node::<C>(child, doc, buf);
            }
        }
        NodeType::ElementNode => {
            if let Some(name) = doc.tag_name(id) {
                buf.push('<');
                buf.push_str(name);
                if let Some(attrs) = doc.attributes(id) {
                    for (attr_name, attr_value) in attrs {
                        buf.push(' ');
                        buf.push_str(attr_name);
                        buf.push_str("=\"");
                        buf.push_str(attr_value);
                        buf.push('"');
                    }
                }
                buf.push('>');

                let children: Vec<NodeId> = doc.children(id).to_vec();
                for child in children {
                    write_node::<C>(child, doc, buf);
                }

                buf.push_str("</");
                buf.push_str(name);
                buf.push('>');
            }
        }
    }
}
