use gosub_html5::node::{Node, NodeData};
use gosub_html5::parser::document::DocumentBuilder;
use gosub_html5::parser::{document::Document, Html5Parser};
use gosub_shared::byte_stream::{ByteStream, Encoding};
use gosub_shared::types::Result;
use std::process::exit;

fn main() -> Result<()> {
    let url = std::env::args()
        .nth(1)
        .or_else(|| {
            println!("Usage: display-text-tree <url>");
            exit(1);
        })
        .unwrap();

    // Fetch the html from the url
    let response = ureq::get(&url).call().map_err(Box::new)?;
    if !response.status() == 200 {
        println!("could not get url. Status code {}", response.status());
        exit(1);
    }
    let html = response.into_string()?;

    let mut stream = ByteStream::new(Encoding::UTF8, None);
    stream.read_from_str(&html, Some(Encoding::UTF8));
    stream.close();

    let document = DocumentBuilder::new_document(None);
    let parse_errors = Html5Parser::parse_document(&mut stream, Document::clone(&document), None)?;

    for e in parse_errors {
        println!("Parse Error: {}", e.message);
    }

    display_node(&document.get(), document.get().get_root());

    Ok(())
}

fn display_node(document: &Document, node: &Node) {
    if let NodeData::Text(text) = &node.data {
        if !text.value().eq("\n") {
            println!("{}", text.value());
        }
    }
    for child_id in &node.children {
        if let Some(child) = document.get_node_by_id(*child_id) {
            display_node(document, child);
        }
    }
}
