use gosub_css3::system::Css3System;
use gosub_html5::document::builder::DocumentBuilderImpl;
use gosub_html5::parser::Html5Parser;
use gosub_shared::byte_stream::{ByteStream, Encoding};
use gosub_shared::types::Result;
use std::process::exit;

use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::document::fragment::DocumentFragmentImpl;
use gosub_interface::config::{HasCssSystem, HasDocument, HasHtmlParser};
use gosub_interface::document::{Document, DocumentBuilder};

use gosub_interface::node::{Node, TextDataType};
use gosub_shared::node::NodeId;

#[derive(Clone, Debug, PartialEq)]
struct Config;

impl HasCssSystem for Config {
    type CssSystem = Css3System;
}
impl HasDocument for Config {
    type Document = DocumentImpl<Self>;
    type DocumentFragment = DocumentFragmentImpl<Self>;
    type DocumentBuilder = DocumentBuilderImpl;
}

impl HasHtmlParser for Config {
    type HtmlParser = Html5Parser<'static, Self>;
}

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

    let mut doc = <DocumentBuilderImpl as DocumentBuilder<Config>>::new_document(None);
    let parse_errors = Html5Parser::<Config>::parse_document(&mut stream, &mut doc, None)?;

    for e in parse_errors {
        println!("Parse Error: {}", e.message);
    }

    display_node::<Config>(&doc, doc.get_root().id());

    Ok(())
}

fn display_node<C: HasDocument>(doc: &C::Document, node_id: NodeId) {
    let node = doc.node_by_id(node_id).unwrap();
    if node.is_text_node() {
        if let Some(data) = node.get_text_data() {
            if !data.value().eq("\n") {
                println!("{}", data.value());
            }
        }
    }

    for &child_id in &node.children().to_vec() {
        display_node::<C>(doc, child_id);
    }
}
