//! Quick manual timing for the HTML5 parser: parses the WHATWG spec (and the
//! synthetic large doc from the benchmarks) once and prints wall-clock times.
//! For statistical numbers use `cargo bench -p gosub_html5 --bench html_parser`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::Read as _;
use std::time::Instant;

use gosub_css3::system::Css3System;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::html_compile;
use gosub_html5::parser::Html5Parser;
use gosub_interface::config::{HasCssSystem, HasDocument, HasHtmlParser};
use gosub_interface::document::Document;

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

fn main() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/whatwg.html.gz");
    let file = std::fs::File::open(&path).expect("failed to open whatwg.html.gz");
    let mut decoder = flate2::read::GzDecoder::new(file);
    let mut html = String::new();
    decoder.read_to_string(&mut html).expect("failed to decompress");

    // Neuter external resource URLs so the parser doesn't do real network fetches
    // (sync stylesheet loading) while we measure parse time.
    let html = html.replace("https://resources.whatwg.org/", "bench://resources.whatwg.org/");

    println!("whatwg spec: {} bytes", html.len());

    let runs: u32 = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(3);
    for run in 1..=runs {
        let start = Instant::now();
        let doc = html_compile::<Config>(&html);
        let elapsed = start.elapsed();
        println!("run {run}: parsed in {elapsed:?} ({} nodes)", std::hint::black_box(&doc).node_count());
    }
}
