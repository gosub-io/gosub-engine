use anyhow::{bail, Result};
use gosub_engine::bytes::{CharIterator, Confidence, Encoding};
use gosub_engine::html5::parser::Html5Parser;
use gosub_engine::html5::parser::document::Document;
use gosub_engine::html5::parser::document::DocumentBuilder;
use std::fs;
use gosub_engine::styles::calculate_styles;

fn main() -> Result<()> {
    let matches = clap::Command::new("Gosub Style parser")
        .version("0.1.0")
        .arg(
            clap::Arg::new("url")
                .help("The url or file to parse")
                .required(true)
                .index(1),
        )
        .get_matches();

    let url: String = matches.get_one::<String>("url").expect("url").to_string();

    let html = if url.starts_with("http://") || url.starts_with("https://") {
        // Fetch the html from the url
        let response = ureq::get(&url).call()?;
        if response.status() != 200 {
            bail!(format!(
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

    let document = DocumentBuilder::new_document();
    let parse_errors = Html5Parser::parse_document(&mut chars, Document::clone(&document), None)?;

    calculate_styles(document, &[]);

    Ok(())
}