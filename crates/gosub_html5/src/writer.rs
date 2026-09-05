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

/// Work stack for [`write_node`].
///
/// This walk used to recurse, which a page can drive as deep as it likes - an inline `<svg>` of
/// nested `<g>` comes back through here on the layout path, on a 2 MiB tokio worker. Keeping the
/// pending work on the heap makes depth cost memory instead of stack.
enum Step {
    /// Emit this node's own text, then queue its children.
    Enter(NodeId),
    /// Emit the closing tag of an element whose children have all been written.
    Close(NodeId),
}

fn write_node<C: HasDocument>(root: NodeId, doc: &C::Document, buf: &mut String) {
    let mut stack = vec![Step::Enter(root)];

    while let Some(step) = stack.pop() {
        let id = match step {
            Step::Close(id) => {
                if let Some(name) = doc.tag_name(id) {
                    buf.push_str("</");
                    buf.push_str(name);
                    buf.push('>');
                }
                continue;
            }
            Step::Enter(id) => id,
        };

        match doc.node_type(id) {
            NodeType::DocumentNode => {}
            NodeType::DocTypeNode => {
                if let Some(name) = doc.doctype_name(id) {
                    buf.push_str("<!DOCTYPE ");
                    buf.push_str(name);
                    buf.push('>');
                }
            }
            NodeType::TextNode => {
                if let Some(value) = doc.text_value(id) {
                    buf.push_str(value);
                }
            }
            NodeType::CommentNode => {
                if let Some(value) = doc.comment_value(id) {
                    buf.push_str("<!--");
                    buf.push_str(value);
                    buf.push_str("-->");
                }
            }
            NodeType::ElementNode => {
                // A nameless element writes nothing, children included, as before: the `if let`
                // used to guard the recursion too.
                let Some(name) = doc.tag_name(id) else {
                    continue;
                };
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
                stack.push(Step::Close(id));
            }
        }

        // Reversed, so they pop back in document order.
        stack.extend(doc.children(id).iter().rev().copied().map(Step::Enter));
    }
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
