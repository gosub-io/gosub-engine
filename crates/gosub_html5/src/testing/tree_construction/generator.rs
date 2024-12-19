use crate::node::HTML_NAMESPACE;
use crate::node::{MATHML_NAMESPACE, SVG_NAMESPACE, XLINK_NAMESPACE, XMLNS_NAMESPACE};
use gosub_shared::document::DocumentHandle;
use gosub_shared::traits::config::HasDocument;
use gosub_shared::traits::document::Document;
use gosub_shared::traits::node::{CommentDataType, DocTypeDataType, ElementDataType, Node, NodeType, TextDataType};

/// Generates a tree output that can be used for matching with the expected output
pub struct TreeOutputGenerator<C: HasDocument> {
    document: DocumentHandle<C>,
}

impl<C: HasDocument> TreeOutputGenerator<C> {
    /// Initializes a new tree output generator
    #[must_use]
    pub fn new(document: DocumentHandle<C>) -> Self {
        Self { document }
    }

    /// Generates a tree
    pub fn generate(&self) -> Vec<String> {
        self.output_treeline(self.document.get().get_root(), 0)
    }

    /// Generates an array of indented tree line and its children. Note that text lines can have newlines in them
    fn output_treeline(&self, node: &C::Node, indent_level: usize) -> Vec<String> {
        let mut indent_level = indent_level;
        let mut output = Vec::new();

        // We can skip the document node, as it is always the root node (either a document node, or
        // a html node when it's a fragment)
        if indent_level > 0 {
            output.push(format!("| {}{}", "  ".repeat(indent_level - 1), self.output_node(node)));

            if node.type_of() == NodeType::ElementNode {
                if let Some(element) = &node.get_element_data() {
                    let mut sorted_attrs = vec![];
                    for attr in element.attributes() {
                        sorted_attrs.push(attr);
                    }
                    sorted_attrs.sort_by(|a, b| a.0.cmp(b.0));

                    for attr in &sorted_attrs {
                        output.push(format!(r#"| {}{}="{}""#, "  ".repeat(indent_level), attr.0, attr.1));
                    }

                    // Template tags have an extra "content" node in the test tree ouput
                    if element.name() == "template" && element.is_namespace(HTML_NAMESPACE) {
                        output.push(format!("| {}content", "  ".repeat(indent_level)));
                        indent_level += 1;
                    }
                }
            }
        }

        for child_id in node.children() {
            let doc = self.document.get();
            let child_node = doc.node_by_id(*child_id).expect("node not found");

            output.append(&mut self.output_treeline(child_node, indent_level + 1));
        }

        output
    }

    /// Generate the output for a single node
    fn output_node(&self, node: &C::Node) -> String {
        match node.type_of() {
            NodeType::ElementNode => {
                let Some(data) = node.get_element_data() else {
                    return "<unknown>".to_owned();
                };

                let ns_prefix = match data.namespace() {
                    MATHML_NAMESPACE => "math ",
                    SVG_NAMESPACE => "svg ",
                    XMLNS_NAMESPACE => "xml ",
                    XLINK_NAMESPACE => "xlink ",
                    _ => "",
                };

                format!("<{}{}>", ns_prefix, data.name())
            }
            NodeType::TextNode => {
                let Some(data) = node.get_text_data() else {
                    return "<unknown>".to_owned();
                };

                format!(r#""{}""#, data.value())
            }
            NodeType::CommentNode => {
                let Some(data) = node.get_comment_data() else {
                    return "unknown".to_owned();
                };

                format!("<!-- {} -->", data.value())
            }
            NodeType::DocTypeNode => {
                let Some(data) = node.get_doctype_data() else {
                    return "<unknown>".to_owned();
                };

                let doctype_text = if data.pub_identifier().is_empty() && data.sys_identifier().is_empty() {
                    // <!DOCTYPE html>
                    data.name()
                } else {
                    // <!DOCTYPE html "pubid" "sysid">
                    &*format!(
                        r#"{0} "{1}" "{2}""#,
                        data.name(),
                        data.pub_identifier(),
                        data.sys_identifier()
                    )
                };

                format!("<!DOCTYPE {}>", doctype_text.trim())
            }
            NodeType::DocumentNode => String::new(),
        }
    }
}
