use anyhow::Result;
use gosub_engine::html5::parser::document::{Document, DocumentBuilder};
use gosub_engine::html5::{
    input_stream::{Confidence, Encoding, InputStream},
    parser::Html5Parser,
};
use std::fs;
use std::process::exit;

fn bail(message: &str) -> ! {
    println!("{}", message);
    exit(1);
}

fn main() -> Result<()> {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| bail("Usage: gosub-parser <url>"));

    let html = if url.starts_with("http://") || url.starts_with("https://") {
        // Fetch the html from the url
        let response = ureq::get(&url).call()?;
        if response.status() != 200 {
            bail(&format!(
                "Could not get url. Status code {}",
                response.status()
            ));
        }
        response.into_string()?
    } else {
        // Get html from the file
        fs::read_to_string(&url)?
    };

    let mut stream = InputStream::new();
    stream.read_from_str(&html, Some(Encoding::UTF8));
    stream.set_confidence(Confidence::Certain);

    // If the encoding confidence is not Confidence::Certain, we should detect the encoding.
    if !stream.is_certain_encoding() {
        stream.detect_encoding()
    }

    let document = DocumentBuilder::new_document();
    let parse_errors = Html5Parser::parse_document(&mut stream, Document::clone(&document), None)?;

    println!("Generated tree: \n\n {}", document);

    for e in parse_errors {
        println!("Parse Error: {}", e.message)
    }

    Ok(())
}
