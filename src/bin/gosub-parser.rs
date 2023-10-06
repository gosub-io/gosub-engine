use std::fs;
use std::process::exit;

use gosub_engine::html5_parser::input_stream::Confidence;
use gosub_engine::html5_parser::input_stream::{Encoding, InputStream};
use gosub_engine::html5_parser::parser::Html5Parser;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::args()
        .nth(1)
        .or_else(|| {
            println!("Usage: gosub-parser <url>");
            exit(1);
        })
        .unwrap();

    let html = if url.starts_with("http://") || url.starts_with("https://") {
        // Fetch the html from the url
        let response = reqwest::blocking::get(&url)?;
        if !response.status().is_success() {
            println!("could not get url. Status code {}", response.status());
            exit(1);
        }
        response.text()?
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

    let mut parser = Html5Parser::new(&mut stream);
    let (document, parse_error) = parser.parse();

    println!("Generated tree: \n\n {}", document);

    for e in parse_error {
        println!("Parse Error: {}", e.message)
    }

    Ok(())
}
