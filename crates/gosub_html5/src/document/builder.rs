use crate::node::HTML_NAMESPACE;
use gosub_interface::config::HasDocument;
use gosub_interface::document::DocumentBuilder;
use gosub_interface::document::{Document, DocumentType};
use gosub_interface::node::{Node, QuirksMode};
use std::collections::HashMap;
use url::Url;

/// This struct will be used to create a fully initialized document or document fragment
pub struct DocumentBuilderImpl {}

impl<C: HasDocument> DocumentBuilder<C> for DocumentBuilderImpl {
    /// Creates a new document with a document root node
    fn new_document(url: Option<Url>) -> C::Document {
        C::Document::new(DocumentType::HTML, url, None)
    }

    /// Creates a new document fragment with the context as the root node
    fn new_document_fragment(context_node: &C::Node, quirks_mode: QuirksMode) -> C::Document {
        // Create a new document with an HTML node as the root node
        let fragment_root_node =
            C::Document::new_element_node("html", Some(HTML_NAMESPACE), HashMap::new(), context_node.location());
        let mut fragment = C::Document::new(DocumentType::HTML, None, Some(fragment_root_node));

        // let context_node = context_node.clone();
        match quirks_mode {
            QuirksMode::Quirks => {
                fragment.set_quirks_mode(QuirksMode::Quirks);
            }
            QuirksMode::LimitedQuirks => {
                fragment.set_quirks_mode(QuirksMode::LimitedQuirks);
            }
            _ => {}
        }

        fragment
    }
}
