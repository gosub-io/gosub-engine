use anyhow::Result;
use gosub_html5::parser::document::{Document, DocumentBuilder};
use gosub_html5::parser::Html5Parser;
use gosub_shared::bytes::{CharIterator, Confidence, Encoding};
use std::fs;
use std::process::exit;
use url::Url;
use std::str::FromStr;

fn bail(message: &str) -> ! {
    println!("{message}");
    exit(1);
}

fn main() -> Result<()> {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| bail("Usage: gosub-parser <url>"));

    let url = Url::from_str(&url).unwrap_or_else(|_| bail("Invalid url"));

    println!("Parsing url: {:?}", url);

    let html = if url.scheme() == "http" || url.scheme() == "https" {
        // Fetch the html from the url
        let response = ureq::get(&url.to_string()).call()?;
        if response.status() != 200 {
            bail(&format!(
                "Could not get url. Status code {}",
                response.status()
            ));
        }
        response.into_string()?
    } else if url.scheme() == "file"{
        // Get html from the file
        fs::read_to_string(&url.to_string())?
    } else {
        bail("Invalid url scheme");
    };

    let mut chars = CharIterator::new();
    chars.read_from_str(&html, Some(Encoding::UTF8));
    chars.set_confidence(Confidence::Certain);

    // If the encoding confidence is not Confidence::Certain, we should detect the encoding.
    if !chars.is_certain_encoding() {
        chars.detect_encoding();
    }

    // SimpleLogger::new().init().unwrap();

    // Create a new document that will be filled in by the parser
    let handle = DocumentBuilder::new_document(Some(url));
    let parse_errors = Html5Parser::parse_document(&mut chars, Document::clone(&handle), None)?;


    println!("Found {} stylesheets", handle.get().stylesheets.len());
    for sheet in &handle.get().stylesheets {
        println!("Stylesheet location: {:?}", sheet.location);
    }

    // let mut handle_mut = handle.get_mut();
    // CssComputer::new(&mut *handle_mut).generate_style();
    // drop(handle_mut);

    // println!("Generated tree: \n\n {handle}");

    for e in parse_errors {
        println!("Parse Error: {}", e.message);
    }

    Ok(())
}


