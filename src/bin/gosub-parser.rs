use anyhow::{bail, Result};
use gosub_html5::parser::document::{Document, DocumentBuilder};
use gosub_html5::parser::Html5Parser;
use gosub_shared::bytes::{CharIterator, Confidence, Encoding};
use std::fs;

fn main() -> Result<()> {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| bail!("Usage: gosub-parser <url>"));

    let html = if url.starts_with("http://") || url.starts_with("https://") {
        // Fetch the html from the url
        let response = ureq::get(&url).call()?;
        if response.status() != 200 {
            bail!(&format!(
                "Could not get url. Status code {}",
                response.status()
            ));
        }
        response.into_string()?
    } else {
        // Get html from the file
        fs::read_to_string(&url)?
    };

    let mut chars = CharIterator::new();
    chars.read_from_str(&html, Some(Encoding::UTF8));
    chars.set_confidence(Confidence::Certain);

    // If the encoding confidence is not Confidence::Certain, we should detect the encoding.
    if !chars.is_certain_encoding() {
        chars.detect_encoding();
    }

    let document = DocumentBuilder::new_document();
    let parse_errors = Html5Parser::parse_document(&mut chars, Document::clone(&document), None)?;

    println!("Generated tree: \n\n {document}");

    for e in parse_errors {
        println!("Parse Error: {}", e.message);
    }

    Ok(())
}
