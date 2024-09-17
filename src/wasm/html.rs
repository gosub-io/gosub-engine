use std::borrow::Borrow;

use url::Url;
use wasm_bindgen::prelude::wasm_bindgen;

use gosub_html5::parser::document::{Document, DocumentBuilder};
use gosub_html5::parser::Html5Parser;
use gosub_shared::byte_stream::{ByteStream, Encoding};

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
    let doc = DocumentBuilder::new_document(url);

    let mut stream = ByteStream::new(Encoding::UTF8, None);
    stream.read_from_str(&input, Some(Encoding::UTF8));
    stream.close();

    let mut errors = String::new();

    match Html5Parser::parse_document(&mut stream, Document::clone(&doc), None) {
        Ok(errs) => {
            for e in errs {
                errors.push_str(&format!("{}@{:?}\n", e.message, e.location));
            }
        }
        Err(e) => {
            errors = format!("Failed to parse HTML: {}", e);
        }
    }

    let out = doc.borrow().to_string();

    HTMLOutput { out, errors }
}
