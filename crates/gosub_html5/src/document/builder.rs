use crate::document::document_impl::DocumentImpl;
use gosub_interface::config::HasDocument;
use gosub_interface::document::{Document, DocumentType};
use gosub_interface::node::QuirksMode;
use url::Url;

pub struct DocumentBuilderImpl;

impl DocumentBuilderImpl {
    pub fn new_document<C: HasDocument<Document = DocumentImpl<C>>>(url: Option<Url>) -> DocumentImpl<C> {
        DocumentImpl::new(DocumentType::HTML, url)
    }

    pub fn new_document_fragment<C: HasDocument<Document = DocumentImpl<C>>>(
        quirks_mode: QuirksMode,
    ) -> DocumentImpl<C> {
        DocumentImpl::new_fragment(quirks_mode)
    }
}
