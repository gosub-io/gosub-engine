use gosub_interface::config::HasDocument;
use gosub_interface::document::{Document, DocumentType};
use gosub_interface::node::QuirksMode;
use url::Url;

pub struct DocumentBuilderImpl;

impl DocumentBuilderImpl {
    pub fn new_document<C: HasDocument>(url: Option<Url>) -> C::Document {
        C::Document::new(DocumentType::HTML, url)
    }

    pub fn new_document_fragment<C: HasDocument>(quirks_mode: QuirksMode) -> C::Document {
        C::Document::new_fragment(quirks_mode)
    }
}
