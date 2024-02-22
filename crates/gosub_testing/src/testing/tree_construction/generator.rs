use gosub_html5::node::{Node, NodeData, NodeTrait, NodeType, HTML_NAMESPACE};
use gosub_html5::node::{MATHML_NAMESPACE, SVG_NAMESPACE, XLINK_NAMESPACE, XMLNS_NAMESPACE};
use gosub_html5::parser::document::DocumentHandle;

/// Generates a tree output that can be used for matching with the expected output
pub struct TreeOutputGenerator {
    document: DocumentHandle,
}

impl TreeOutputGenerator {
    /// Initializes a new tree output generator
    #[must_use]
    pub fn new(document: DocumentHandle) -> Self {
        Self { document }
    }

    /// Generates a tree
    pub fn generate(&self) -> Vec<String> {
        self.output_treeline(self.document.get().get_root(), 0)
    }

    /// Generates an array of indented tree line and its children. Note that text lines can have newlines in them
    fn output_treeline(&self, node: &Node, indent_level: usize) -> Vec<String> {
        let mut indent_level = indent_level;
        let mut output = Vec::new();

        // We can skip the document node, as it is always the root node (either a document node, or
        // a html node when it's a fragment)
        if indent_level > 0 {
            output.push(format!(
                "| {}{}",
                "  ".repeat(indent_level - 1),
                self.output_node(node)
            ));

            if node.type_of() == NodeType::Element {
                if let NodeData::Element(element) = &node.data {
                    let mut sorted_attrs = vec![];
                    for attr in &element.attributes {
                        sorted_attrs.push(attr);
                    }
                    sorted_attrs.sort_by(|a, b| a.0.cmp(b.0));

                    for attr in &sorted_attrs {
                        output.push(format!(
                            r#"| {}{}="{}""#,
                            "  ".repeat(indent_level),
                            attr.0,
                            attr.1
                        ));
                    }
                }
            }

            // Template tags have an extra "content" node in the test tree ouput
            if node.name == "template" && node.is_namespace(HTML_NAMESPACE) {
                output.push(format!("| {}content", "  ".repeat(indent_level)));
                indent_level += 1;
            }
        }

        for child_id in &node.children {
            let doc = self.document.get();
            let child_node = doc.get_node_by_id(*child_id).expect("node not found");

            output.append(&mut self.output_treeline(child_node, indent_level + 1));
        }

        output
    }

    /// Generate the output for a single node
    fn output_node(&self, node: &Node) -> String {
        match node.data.clone() {
            NodeData::Element(element) => {
                if let Some(ns) = node.namespace.clone() {
                    let ns_prefix = match ns.as_str() {
                        MATHML_NAMESPACE => "math ",
                        SVG_NAMESPACE => "svg ",
                        XMLNS_NAMESPACE => "xml ",
                        XLINK_NAMESPACE => "xlink ",
                        _ => "",
                    };
                    format!("<{}{}>", ns_prefix, element.name())
                } else {
                    // format!("<{}{}>", ns_prefix, element.name())
                    format!("<{}>", element.name())
                }
            }
            NodeData::Text(text) => format!(r#""{}""#, text.value()),
            NodeData::Comment(comment) => format!("<!-- {} -->", comment.value()),
            NodeData::DocType(doctype) => {
                let doctype_text =
                    if doctype.pub_identifier.is_empty() && doctype.sys_identifier.is_empty() {
                        // <!DOCTYPE html>
                        doctype.name
                    } else {
                        // <!DOCTYPE html "pubid" "sysid">
                        format!(
                            r#"{0} "{1}" "{2}""#,
                            doctype.name, doctype.pub_identifier, doctype.sys_identifier
                        )
                    };

                format!("<!DOCTYPE {}>", doctype_text.trim())
            }
            NodeData::Document(_) => String::new(),
        }
    }
}
