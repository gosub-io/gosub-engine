use anyhow::bail;
use gosub_html5::parser::document::{Document, DocumentBuilder};
use gosub_html5::parser::Html5Parser;
use gosub_shared::byte_stream::{ByteStream, Encoding};
use gosub_shared::timing::Scale;
use gosub_shared::timing_display;
use gosub_shared::types::Result;
use std::fs;
use std::process::exit;
use std::str::FromStr;
use url::Url;

fn bail(message: &str) -> ! {
    println!("{message}");
    exit(1);
}

fn main() -> Result<()> {
    let matches = clap::Command::new("Gosub parser")
        .version("0.1.0")
        .arg(
            clap::Arg::new("url")
                .help("The url or file to parse")
                .required(true)
                .index(1),
        )
        .get_matches();

    let url = matches
        .get_one::<String>("url")
        .ok_or("Missing url")
        .unwrap()
        .to_string();

    let url = Url::from_str(&url).unwrap_or_else(|_| bail("Invalid url"));

    println!("Parsing url: {:?}", url);

    let html = if url.scheme() == "http" || url.scheme() == "https" {
        // Fetch the html from the url
        let response = ureq::get(url.as_ref()).call()?;
        if response.status() != 200 {
            bail!("Could not get url. Status code {}", response.status());
        }
        response.into_string()?
    } else if url.scheme() == "file" {
        // Get html from the file
        fs::read_to_string(url.to_string().trim_start_matches("file://"))?
    } else {
        bail("Invalid url scheme");
    };

    let mut stream = ByteStream::new(Encoding::UTF8, None);
    stream.read_from_str(&html, Some(Encoding::UTF8));
    stream.close();

    // SimpleLogger::new().init().unwrap();

    // Create a new document that will be filled in by the parser
    let handle = DocumentBuilder::new_document(Some(url));
    let parse_errors = Html5Parser::parse_document(&mut stream, Document::clone(&handle), None)?;

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

    timing_display!(true, Scale::Auto);

    Ok(())
}
