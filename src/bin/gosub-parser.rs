use anyhow::bail;
use gosub_css3::system::Css3System;
use gosub_html5::document::builder::DocumentBuilderImpl;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::parser::Html5Parser;
use gosub_interface::config::{HasCssSystem, HasDocument, HasHtmlParser};
use gosub_shared::byte_stream::{ByteStream, Encoding};
use gosub_shared::timing::Scale;
use gosub_shared::timing_display;
use gosub_shared::types::Result;
use std::process::exit;
use std::str::FromStr;
use url::Url;

fn fatal(message: &str) -> ! {
    println!("{message}");
    exit(1);
}
#[derive(Clone, Debug, PartialEq)]
struct Config;

impl HasCssSystem for Config {
    type CssSystem = Css3System;
}
impl HasDocument for Config {
    type Document = DocumentImpl<Self>;
}

impl HasHtmlParser for Config {
    type HtmlParser = Html5Parser<'static, Self>;
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

    let url = Url::from_str(&url).unwrap_or_else(|_| fatal("Invalid url"));

    println!("Parsing url: {url:?}");

    let response = gosub_net::http::blocking::get(&url)?;
    if !response.is_ok() {
        bail!("Could not get url. Status code {}", response.status);
    }
    let html = String::from_utf8_lossy(&response.body).into_owned();

    let mut stream = ByteStream::from_str(&html, Encoding::UTF8);

    // SimpleLogger::new().init().unwrap();

    // Create a new document that will be filled in by the parser
    let mut doc = DocumentBuilderImpl::new_document::<Config>(Some(url));
    let parse_errors = Html5Parser::<Config>::parse_document(&mut stream, &mut doc, None)?;

    println!("Found {} stylesheets", doc.stylesheets.len());
    for sheet in &doc.stylesheets {
        println!("Stylesheet url: {:?}", sheet.url);
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
