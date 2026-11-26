//! The text of parse errors that name the tag they are about.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use gosub_css3::system::Css3System;
use gosub_html5::document::builder::DocumentBuilderImpl;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::parser::Html5Parser;
use gosub_interface::config::ModuleConfiguration;
use gosub_shared::byte_stream::{ByteStream, Encoding};

#[derive(Clone, Debug, PartialEq)]
struct Config;

impl ModuleConfiguration for Config {
    type CssSystem = Css3System;
    type Document = DocumentImpl<Self>;
    type HtmlParser = Html5Parser<'static, Self>;
}

fn errors(html: &str) -> Vec<String> {
    let mut stream = ByteStream::from_str(html, Encoding::UTF8);
    let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
    Html5Parser::<Config>::parse_document(&mut stream, &mut doc, None)
        .unwrap()
        .into_iter()
        .map(|e| e.message)
        .collect()
}

#[test]
fn a_dt_that_closes_an_open_dt_names_it() {
    // The div is not closed by the implied end tags, so the open dt is not the current node.
    let errors = errors("<!doctype html><dt><div><dt>");
    assert!(errors.iter().any(|e| e == "dt tag not at top of stack"), "{errors:?}");
}

#[test]
fn a_dd_that_closes_an_open_dd_names_it() {
    let errors = errors("<!doctype html><dd><div><dd>");
    assert!(errors.iter().any(|e| e == "dd tag not at top of stack"), "{errors:?}");
}

#[test]
fn a_dd_that_closes_an_open_dt_names_the_dt() {
    let errors = errors("<!doctype html><dt><div><dd>");
    assert!(errors.iter().any(|e| e == "dt tag not at top of stack"), "{errors:?}");
}

#[test]
fn no_error_when_the_open_dt_is_the_current_node() {
    let errors = errors("<!doctype html><dt>x<dt>");
    assert!(!errors.iter().any(|e| e.contains("not at top of stack")), "{errors:?}");
}
