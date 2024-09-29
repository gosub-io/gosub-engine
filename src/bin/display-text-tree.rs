use gosub_css3::system::Css3System;
use gosub_html5::document::builder::DocumentBuilderImpl;
use gosub_html5::parser::Html5Parser;
use gosub_shared::byte_stream::{ByteStream, Encoding};
use gosub_shared::types::Result;
use std::process::exit;

use gosub_html5::document::document_impl::DocumentImpl;
use gosub_shared::document::DocumentHandle;
use gosub_shared::node::NodeId;
use gosub_shared::traits::css3::CssSystem;
use gosub_shared::traits::document::{Document, DocumentBuilder};
use gosub_shared::traits::node::Node;

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

    let doc_handle: DocumentHandle<DocumentImpl<Css3System>, Css3System> = DocumentBuilderImpl::new_document(None);
    let parse_errors =
        Html5Parser::<DocumentImpl<Css3System>, Css3System>::parse_document(&mut stream, doc_handle.clone(), None)?;

    for e in parse_errors {
        println!("Parse Error: {}", e.message);
    }

    display_node::<DocumentImpl<Css3System>, Css3System>(doc_handle.clone(), doc_handle.get().get_root().id());

    Ok(())
}

fn display_node<D: Document<C>, C: CssSystem>(
    doc_handle: DocumentHandle<DocumentImpl<Css3System>, Css3System>,
    node_id: NodeId,
) {
    let binding = doc_handle.get();
    let node = binding.node_by_id(node_id).unwrap();
    if node.is_text_node() {
        if let Some(data) = node.get_text_data() {
            if !data.value.eq("\n") {
                println!("{}", data.value);
            }
        }
    }

    for &child_id in &node.children().to_vec() {
        display_node::<D, C>(doc_handle.clone(), child_id);
    }
}
