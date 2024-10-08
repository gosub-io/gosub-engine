use gosub_css3::system::Css3System;
use gosub_html5::document::builder::DocumentBuilderImpl;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::parser::Html5Parser;
use gosub_html5::writer::DocumentWriter;
use gosub_shared::byte_stream::{ByteStream, Encoding};
use gosub_shared::document::DocumentHandle;
use gosub_shared::node::NodeId;
use gosub_shared::traits::document::DocumentBuilder;
use url::Url;
use wasm_bindgen::prelude::wasm_bindgen;

#[wasm_bindgen]
pub struct HTMLOptions {
    url: String,
}

#[wasm_bindgen]
impl HTMLOptions {
    #[wasm_bindgen(constructor)]
    pub fn new(url: String) -> Self {
        Self { url }
    }
}

#[wasm_bindgen]
pub struct HTMLOutput {
    out: String,
    errors: String,
}

#[wasm_bindgen]
impl HTMLOutput {
    pub fn to_string(&self) -> String {
        format!("{}\n{}", self.out, self.errors)
    }

    pub fn out(&self) -> String {
        self.out.clone()
    }

    pub fn errors(&self) -> String {
        self.errors.clone()
    }
}

#[wasm_bindgen]
pub fn html_parser(input: &str, opts: HTMLOptions) -> HTMLOutput {
    let url = Url::parse(&opts.url).ok();
    let doc: DocumentHandle<DocumentImpl<Css3System>, Css3System> = DocumentBuilderImpl::new_document(url);

    let mut stream = ByteStream::new(Encoding::UTF8, None);
    stream.read_from_str(&input, Some(Encoding::UTF8));
    stream.close();

    let mut errors = String::new();

    match Html5Parser::parse_document(&mut stream, DocumentHandle::clone(&doc), None) {
        Ok(errs) => {
            for e in errs {
                errors.push_str(&format!("{}@{:?}\n", e.message, e.location));
            }
        }
        Err(e) => {
            errors = format!("Failed to parse HTML: {}", e);
        }
    }

    let out = DocumentWriter::write_from_node(NodeId::root(), doc);

    HTMLOutput { out, errors }
}
