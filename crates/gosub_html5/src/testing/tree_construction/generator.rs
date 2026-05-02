use crate::node::{MATHML_NAMESPACE, SVG_NAMESPACE, XLINK_NAMESPACE, XMLNS_NAMESPACE};
use gosub_interface::config::HasDocument;
use gosub_interface::document::Document;
use gosub_interface::node::NodeType;
use gosub_shared::node::NodeId;

/// Generates a tree output that can be used for matching with the expected output
pub struct TreeOutputGenerator<C: HasDocument> {
    document: C::Document,
}

impl<C: HasDocument> TreeOutputGenerator<C> {
    /// Initializes a new tree output generator
    #[must_use]
    pub fn new(document: C::Document) -> Self {
        Self { document }
    }

    /// Generates a tree
    pub fn generate(&self) -> Vec<String> {
        self.output_treeline(self.document.root(), 0)
    }

    /// Generates an array of indented tree lines for a node and its children.
    fn output_treeline(&self, node_id: NodeId, indent_level: usize) -> Vec<String> {
        let mut indent_level = indent_level;
        let mut output = Vec::new();

        // Skip the document root — it's always the implicit container
        if indent_level > 0 {
            output.push(format!(
                "| {}{}",
                "  ".repeat(indent_level - 1),
                self.output_node(node_id)
            ));

            if self.document.node_type(node_id) == NodeType::ElementNode {
                if let Some(attrs) = self.document.attributes(node_id) {
                    let mut sorted_attrs: Vec<(&String, &String)> = attrs.iter().collect();
                    sorted_attrs.sort_by(|a, b| a.0.cmp(b.0));

                    for (key, value) in &sorted_attrs {
                        output.push(format!(r#"| {}{}="{}""#, "  ".repeat(indent_level), key, value));
                    }
                }

                // Template tags have an extra "content" node in the test tree output
                let is_html_template = self.document.tag_name(node_id) == Some("template")
                    && self.document.namespace(node_id) == Some(crate::node::HTML_NAMESPACE);
                if is_html_template {
                    output.push(format!("| {}content", "  ".repeat(indent_level)));
                    indent_level += 1;
                }
            }
        }

        for child_id in self.document.children(node_id).to_vec() {
            output.append(&mut self.output_treeline(child_id, indent_level + 1));
        }

        output
    }

    /// Generate the output string for a single node
    fn output_node(&self, node_id: NodeId) -> String {
        match self.document.node_type(node_id) {
            NodeType::ElementNode => {
                let name = self.document.tag_name(node_id).unwrap_or("<unknown>");
                let ns_prefix = match self.document.namespace(node_id).unwrap_or("") {
                    MATHML_NAMESPACE => "math ",
                    SVG_NAMESPACE => "svg ",
                    XMLNS_NAMESPACE => "xml ",
                    XLINK_NAMESPACE => "xlink ",
                    _ => "",
                };
                format!("<{}{}>", ns_prefix, name)
            }
            NodeType::TextNode => {
                let value = self.document.text_value(node_id).unwrap_or("<unknown>");
                format!(r#""{}""#, value)
            }
            NodeType::CommentNode => {
                let value = self.document.comment_value(node_id).unwrap_or("");
                format!("<!-- {} -->", value)
            }
            NodeType::DocTypeNode => {
                let name = self.document.doctype_name(node_id).unwrap_or("");
                let pub_id = self.document.doctype_public_id(node_id).unwrap_or("");
                let sys_id = self.document.doctype_system_id(node_id).unwrap_or("");
                let doctype_text = if pub_id.is_empty() && sys_id.is_empty() {
                    name.to_owned()
                } else {
                    format!(r#"{name} "{pub_id}" "{sys_id}""#)
                };
                format!("<!DOCTYPE {}>", doctype_text.trim())
            }
            NodeType::DocumentNode => String::new(),
        }
    }
}
