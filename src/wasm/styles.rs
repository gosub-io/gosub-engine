use gosub_css3::system::Css3System;
use gosub_html5::document::builder::DocumentBuilderImpl;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::parser::Html5Parser;
use gosub_interface::document::{Document, DocumentBuilder};
use gosub_shared::byte_stream::{ByteStream, Encoding};
use url::Url;
use wasm_bindgen::prelude::wasm_bindgen;

#[wasm_bindgen]
pub struct StylesOptions {
    url: String,
}

#[wasm_bindgen]
impl StylesOptions {
    #[wasm_bindgen(constructor)]
    pub fn new(url: String) -> Self {
        Self { url }
    }
}

#[wasm_bindgen]
pub struct StylesOutput {
    errors: String,
    document_dump: String,
}

#[wasm_bindgen]
impl StylesOutput {
    pub fn to_string(&self) -> String {
        format!("{}\n{}", self.document_dump, self.errors)
    }

    pub fn document_dump(&self) -> String {
        self.document_dump.clone()
    }

    pub fn errors(&self) -> String {
        self.errors.clone()
    }
}

#[wasm_bindgen]
pub fn styles_parser(input: &str, opts: StylesOptions) -> StylesOutput {
    let url = Url::parse(&opts.url).ok();
    let doc: gosub_interface::document::DocumentHandle<DocumentImpl<Css3System>, Css3System> =
        DocumentBuilderImpl::new_document(url);

    let mut stream = ByteStream::from_str(input, Encoding::UTF8);
    let mut errors = String::new();

    match Html5Parser::parse_document(
        &mut stream,
        gosub_interface::document::DocumentHandle::clone(&doc),
        None,
    ) {
        Ok(errs) => {
            for e in errs {
                errors.push_str(&format!("{}@{:?}\n", e.message, e.location));
            }
        }
        Err(e) => {
            errors = format!("Failed to parse HTML: {}", e);
        }
    }

    // Walk the document and emit a compact style dump per element.
    let document_dump = dump_styles(&doc);

    StylesOutput { errors, document_dump }
}

fn dump_styles(doc: &gosub_interface::document::DocumentHandle<DocumentImpl<Css3System>, Css3System>) -> String {
    use gosub_interface::node::Node;

    let mut out = String::new();
    let doc_read = doc.get();

    let mut stack = vec![(gosub_shared::node::NodeId::root(), 0usize)];
    while let Some((id, depth)) = stack.pop() {
        let indent = "  ".repeat(depth);

        let Some(node) = doc_read.node_by_id(id) else {
            continue;
        };

        match node.type_of() {
            gosub_interface::node::NodeType::ElementNode => {
                let tag = node.as_element_data().map(|e| e.name()).unwrap_or("?");
                out.push_str(&format!("{}<{}", indent, tag));

                // Emit computed CSS properties if any are attached.
                if let Some(props) = node.as_element_data().and_then(|e| e.properties()) {
                    if !props.is_empty() {
                        out.push_str(" [");
                        let mut first = true;
                        for (k, v) in props.iter() {
                            if !first {
                                out.push_str(", ");
                            }
                            out.push_str(&format!("{}: {:?}", k, v));
                            first = false;
                        }
                        out.push(']');
                    }
                }
                out.push_str(">\n");
            }
            gosub_interface::node::NodeType::TextNode => {
                let text = node.as_text_data().map(|t| t.value()).unwrap_or("").trim().to_string();
                if !text.is_empty() {
                    let truncated = if text.len() > 60 {
                        format!("{}…", &text[..60])
                    } else {
                        text
                    };
                    out.push_str(&format!("{}\"{}\"\n", indent, truncated));
                }
            }
            _ => {}
        }

        // Push children in reverse so the first child is processed first.
        let children: Vec<_> = doc_read.child_node_ids(id).collect();
        for child in children.into_iter().rev() {
            stack.push((child, depth + 1));
        }
    }

    out
}
