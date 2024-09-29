use std::collections::HashMap;

use gosub_shared::traits::css3::CssSystem;
use url::Url;

use crate::document::document_impl::DocumentImpl;
use crate::node::HTML_NAMESPACE;
use crate::DocumentHandle;
use gosub_shared::traits::document::DocumentBuilder;
use gosub_shared::traits::document::{Document, DocumentType};
use gosub_shared::traits::node::{Node, QuirksMode};

/// This struct will be used to create a fully initialized document or document fragment
pub struct DocumentBuilderImpl {}

impl<C: CssSystem> DocumentBuilder<C> for DocumentBuilderImpl {
    type Document = DocumentImpl<C>;

    /// Creates a new document with a document root node
    fn new_document(url: Option<Url>) -> DocumentHandle<Self::Document, C> {
        <Self::Document as Document<C>>::new(DocumentType::HTML, url, None)
    }

    /// Creates a new document fragment with the context as the root node
    fn new_document_fragment(
        context_node: &<Self::Document as Document<C>>::Node,
        quirks_mode: QuirksMode,
    ) -> DocumentHandle<Self::Document, C> {
        let handle = context_node.handle();

        // Create a new document with an HTML node as the root node
        let fragment_root_node = <Self::Document as Document<C>>::new_element_node(
            handle.clone(),
            "html",
            Some(HTML_NAMESPACE),
            HashMap::new(),
            context_node.location(),
        );
        let mut fragment_handle =
            <Self::Document as Document<C>>::new(DocumentType::HTML, None, Some(fragment_root_node));

        // let context_node = context_node.clone();
        match quirks_mode {
            QuirksMode::Quirks => {
                fragment_handle.get_mut().set_quirks_mode(QuirksMode::Quirks);
            }
            QuirksMode::LimitedQuirks => {
                fragment_handle.get_mut().set_quirks_mode(QuirksMode::LimitedQuirks);
            }
            _ => {}
        }

        fragment_handle
    }
}
