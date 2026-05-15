use gosub_css3::system::Css3System;
use gosub_html5::document::builder::DocumentBuilderImpl;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::parser::Html5Parser;
use gosub_shared::byte_stream::{ByteStream, Encoding};
use gosub_shared::types::Result;
use std::process::exit;

use gosub_interface::config::{HasCssSystem, HasDocument, HasHtmlParser};
use gosub_interface::document::Document;
use gosub_interface::node::NodeType;
use gosub_shared::node::NodeId;

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
    let url = std::env::args()
        .nth(1)
        .or_else(|| {
            println!("Usage: display-text-tree <url>");
            exit(1);
        })
        .unwrap();

    // Fetch the html from the url
    let parsed_url = url::Url::parse(&url)?;
    let response = gosub_net::net::simple::sync_fetch(&parsed_url)?;
    if !response.is_ok() {
        println!("could not get url. Status code {}", response.status);
        exit(1);
    }
    let html = String::from_utf8_lossy(&response.body).into_owned();

    let mut stream = ByteStream::from_str(&html, Encoding::UTF8);

    let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
    let parse_errors = Html5Parser::<Config>::parse_document(&mut stream, &mut doc, None)?;

    for e in parse_errors {
        println!("Parse Error: {}", e.message);
    }

    display_node::<Config>(&doc, doc.root());

    Ok(())
}

fn display_node<C: HasDocument>(doc: &C::Document, node_id: NodeId) {
    if doc.node_type(node_id) == NodeType::TextNode {
        if let Some(value) = doc.text_value(node_id) {
            if value != "\n" {
                println!("{value}");
            }
        }
    }

    for child_id in doc.children(node_id).to_vec() {
        display_node::<C>(doc, child_id);
    }
}
