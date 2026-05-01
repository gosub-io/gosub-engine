use gosub_interface::config::HasDocument;
use gosub_interface::document::{Document, DocumentType};
use url::Url;

pub struct DocumentBuilderImpl;

impl DocumentBuilderImpl {
    pub fn new_document<C: HasDocument>(url: Option<Url>) -> C::Document {
        C::Document::new(DocumentType::HTML, url)
    }
}
