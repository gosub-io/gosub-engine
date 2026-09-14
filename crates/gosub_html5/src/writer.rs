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

/// Write an attribute name back out as source text.
///
/// "Adjust foreign attributes" (HTML §13.2.6.1) splits a namespaced attribute on a `<svg>` or
/// `<math>` element into a prefix and a local name, and the parser stores the pair space-joined
/// (`xlink:href` becomes `xlink href`, `xmlns` becomes `xmlns ` with an empty local name) because
/// that is the form the html5lib tree-construction tests expect. An attribute name can never
/// contain a space otherwise - the tokenizer ends the name there - so a space unambiguously marks
/// that pair, and serializing it verbatim would emit `xlink href="..."`, which is not well-formed.
/// Rejoin it with a colon.
fn write_attribute_name(name: &str, buf: &mut String) {
    match name.split_once(' ') {
        Some((prefix, "")) => buf.push_str(prefix),
        Some((prefix, local)) => {
            buf.push_str(prefix);
            buf.push(':');
            buf.push_str(local);
        }
        None => buf.push_str(name),
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
                        write_attribute_name(attr_name, buf);
                        buf.push_str("=\"");
                        buf.push_str(attr_value);
                        buf.push('"');
                    }
                }
                buf.push('>');

                // A shadow host's tree is written back out as the declarative template that
                // produced it, ahead of the light children - the form that reparses into the
                // same document. `getHTML()` emits only shadow roots flagged serializable;
                // this writer is a round-trip of the whole tree, so it emits every one.
                if let Some(shadow_root) = doc.shadow_root(id) {
                    write_shadow_root::<C>(shadow_root, doc, buf);
                }

                let children: Vec<NodeId> = doc.children(id).to_vec();
                for child in children {
                    write_node::<C>(child, doc, buf);
                }

                buf.push_str("</");
                buf.push_str(name);
                buf.push('>');
            }
        }
        // Only reachable through a host's side pointer, which `write_shadow_root` follows.
        NodeType::ShadowRootNode => {}
    }
}

/// Writes a shadow root as the `<template shadowrootmode=...>` that declares it.
fn write_shadow_root<C: HasDocument>(id: NodeId, doc: &C::Document, buf: &mut String) {
    let Some(init) = doc.shadow_root_init(id) else {
        return;
    };

    buf.push_str("<template shadowrootmode=\"");
    buf.push_str(init.mode.as_attribute());
    buf.push('"');
    // The remaining declarative attributes are boolean; absent means the default.
    if init.delegates_focus {
        buf.push_str(" shadowrootdelegatesfocus=\"\"");
    }
    if init.clonable {
        buf.push_str(" shadowrootclonable=\"\"");
    }
    if init.serializable {
        buf.push_str(" shadowrootserializable=\"\"");
    }
    buf.push('>');

    for child in doc.children(id).to_vec() {
        write_node::<C>(child, doc, buf);
    }

    buf.push_str("</template>");
}

#[cfg(test)]
mod tests {
    use super::write_attribute_name;

    fn name(input: &str) -> String {
        let mut buf = String::new();
        write_attribute_name(input, &mut buf);
        buf
    }

    #[test]
    fn adjusted_foreign_attributes_are_rejoined_with_a_colon() {
        // These are stored space-joined by "adjust foreign attributes". Writing them verbatim
        // produced `xlink href="..."`, which no XML parser accepts - which is why an inline
        // <svg> failed with "expected '=' ...".
        assert_eq!(name("xlink href"), "xlink:href");
        assert_eq!(name("xml space"), "xml:space");
        assert_eq!(name("xmlns xlink"), "xmlns:xlink");
    }

    #[test]
    fn an_empty_local_name_keeps_only_the_prefix() {
        // `xmlns` is stored as ("xmlns", ""), so the trailing space must not survive.
        assert_eq!(name("xmlns "), "xmlns");
    }

    #[test]
    fn ordinary_attribute_names_are_untouched() {
        assert_eq!(name("viewBox"), "viewBox");
        assert_eq!(name("data-foo"), "data-foo");
    }
}
