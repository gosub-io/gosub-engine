//! An inline `<svg>` reaches the SVG decoder by serializing its subtree back to text, so the
//! serializer runs on the layout path over a subtree of the page's choosing. It used to recurse,
//! and overflowed on documents the parser itself built happily - before the decoder, so before
//! any of the SVG depth limits could apply. Part of GHSA-c762-mxfh-vwvp.
//!
//! Unoptimised and on a small stack, like the pipeline's SVG tests.

use gosub_css3::system::Css3System;
use gosub_html5::document::builder::DocumentBuilderImpl;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::parser::Html5Parser;
use gosub_interface::config::ModuleConfiguration;
use gosub_interface::document::Document;
use gosub_shared::byte_stream::{ByteStream, Encoding};
use gosub_shared::node::NodeId;

#[derive(Clone, Debug, PartialEq)]
struct Config;

impl ModuleConfiguration for Config {
    type CssSystem = Css3System;
    type Document = DocumentImpl<Self>;
    type HtmlParser = Html5Parser<'static, Self>;
}

const REALISTIC_STACK: usize = 2 * 1024 * 1024;

#[test]
fn a_deeply_nested_document_serializes() {
    // Well past the ~5000 that used to abort. Tree construction is iterative, so nothing upstream
    // rejects the document first.
    const DEPTH: usize = 50_000;

    let mut html = String::from("<svg>");
    html.push_str(&"<g>".repeat(DEPTH));
    html.push_str("<rect>");
    html.push_str(&"</g>".repeat(DEPTH));
    html.push_str("</svg>");

    let written = std::thread::Builder::new()
        .stack_size(REALISTIC_STACK)
        .spawn(move || {
            let mut stream = ByteStream::from_str(&html, Encoding::UTF8);
            let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
            let _ = Html5Parser::<Config>::parse_document(&mut stream, &mut doc, None);
            doc.write_from_node(NodeId::root())
        })
        .expect("spawn")
        .join()
        .expect("serializing must not abort the process");

    // The whole subtree was written, not truncated by some depth cut-off.
    assert_eq!(written.matches("<g>").count(), DEPTH);
    assert_eq!(written.matches("</g>").count(), DEPTH);
}

#[test]
fn ordinary_documents_serialize_unchanged() {
    let mut stream = ByteStream::from_str(
        "<html><head></head><body><p class=\"x\">hello<!-- c --><br></p></body></html>",
        Encoding::UTF8,
    );
    let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
    let _ = Html5Parser::<Config>::parse_document(&mut stream, &mut doc, None);

    assert_eq!(
        doc.write_from_node(NodeId::root()),
        "<html><head></head><body><p class=\"x\">hello<!-- c --><br></br></p></body></html>"
    );
}
