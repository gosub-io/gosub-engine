//! An element with very many attributes tokenizes in linear time and keeps them in source order
//! (GOS-300). The duplicate check and every lookup go through `Attributes`, an `IndexMap`; when it
//! was a list each new attribute was compared against all the earlier ones.

#![allow(clippy::unwrap_used, clippy::expect_used)] // a test that cannot set up must fail loudly

use std::time::{Duration, Instant};

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

/// `<div a0="0" ... a{n-1}="{n-1}" a0="dup">`: the trailing duplicate makes every check count.
fn many_attributes(n: usize) -> String {
    let mut html = String::from("<div");
    for i in 0..n {
        html.push_str(&format!(" a{i}=\"{i}\""));
    }
    html.push_str(" a0=\"dup\"></div>");
    html
}

fn parse(html: &str) -> DocumentImpl<Config> {
    let mut stream = ByteStream::from_str(html, Encoding::UTF8);
    let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
    let _ = Html5Parser::<Config>::parse_document(&mut stream, &mut doc, None);
    doc
}

fn find(doc: &DocumentImpl<Config>, id: NodeId, tag: &str) -> Option<NodeId> {
    if doc.tag_name(id) == Some(tag) {
        return Some(id);
    }
    doc.children(id).iter().find_map(|&c| find(doc, c, tag))
}

/// Best of three, so one slow run on a busy machine does not decide the ratio.
fn parse_time(n: usize) -> Duration {
    let html = many_attributes(n);
    (0..3)
        .map(|_| {
            let start = Instant::now();
            parse(&html);
            start.elapsed()
        })
        .min()
        .unwrap()
}

#[test]
fn many_attributes_keep_source_order_and_the_first_duplicate() {
    const N: usize = 100_000;
    let doc = parse(&many_attributes(N));
    let div = find(&doc, NodeId::root(), "div").expect("a div");
    let attributes = doc.attributes(div).expect("attributes");
    assert_eq!(attributes.len(), N);
    for (i, (name, value)) in attributes.iter().enumerate() {
        assert_eq!(
            (name.as_str(), value.as_str()),
            (format!("a{i}").as_str(), i.to_string().as_str())
        );
    }
}

/// Four times the attributes takes about four times as long. Quadratic would be sixteen.
#[test]
fn many_attributes_tokenize_in_linear_time() {
    let small = parse_time(25_000);
    let large = parse_time(100_000);
    let ratio = large.as_secs_f64() / small.as_secs_f64();
    eprintln!("25k: {small:?}, 100k: {large:?}, ratio {ratio:.1}");
    assert!(
        ratio < 8.0,
        "4x the attributes took {ratio:.1}x as long ({small:?} -> {large:?})"
    );
}
