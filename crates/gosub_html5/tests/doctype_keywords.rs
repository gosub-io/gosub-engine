//! The DOCTYPE, PUBLIC and SYSTEM keywords are matched ASCII case-insensitively. Non-ASCII
//! characters whose Unicode uppercase is an ASCII letter (U+017F and U+0131) must not match.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use gosub_css3::system::Css3System;
use gosub_html5::document::builder::DocumentBuilderImpl;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::parser::errors::ErrorLogger;
use gosub_html5::parser::Html5Parser;
use gosub_html5::tokenizer::token::Token;
use gosub_html5::tokenizer::{ParserData, Tokenizer};
use gosub_interface::config::ModuleConfiguration;
use gosub_interface::document::Document;
use gosub_interface::node::QuirksMode;
use gosub_shared::byte_stream::{ByteStream, Encoding, Location};
use std::cell::RefCell;
use std::rc::Rc;
use test_case::test_case;

#[derive(Clone, Debug, PartialEq)]
struct Config;

impl ModuleConfiguration for Config {
    type CssSystem = Css3System;
    type Document = DocumentImpl<Self>;
    type HtmlParser = Html5Parser<'static, Self>;
}

/// Returns (name, force_quirks, public id, system id) of the first token, which must be a DOCTYPE.
fn doctype(html: &str) -> (Option<String>, bool, Option<String>, Option<String>) {
    let mut stream = ByteStream::from_str(html, Encoding::UTF8);
    let error_logger = Rc::new(RefCell::new(ErrorLogger::new()));
    let mut tokenizer = Tokenizer::new(&mut stream, None, error_logger, Location::default());

    match tokenizer.next_token(ParserData::default()).unwrap() {
        Token::DocType {
            name,
            force_quirks,
            pub_identifier,
            sys_identifier,
            ..
        } => (name, force_quirks, pub_identifier, sys_identifier),
        token => panic!("expected a DOCTYPE token, got {token:?}"),
    }
}

fn quirks_mode(html: &str) -> QuirksMode {
    let mut stream = ByteStream::from_str(html, Encoding::UTF8);
    let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
    let _ = Html5Parser::<Config>::parse_document(&mut stream, &mut doc, None);
    doc.quirks_mode()
}

fn s(v: &str) -> Option<String> {
    Some(v.to_string())
}

// U+017F (long s) uppercases to 'S'
#[test_case("<!DOCTYPE html \u{17F}ystem \"x\">" ; "long s first in system")]
#[test_case("<!DOCTYPE html SY\u{17F}TEM \"x\">" ; "long s inside system")]
// U+0131 (dotless i) uppercases to 'I'
#[test_case("<!DOCTYPE html publ\u{131}c \"x\">" ; "dotless i in public")]
#[test_case("<!DOCTYPE html PUBL\u{131}C \"x\">" ; "dotless i in upper public")]
// These do not uppercase to ASCII, but must not match either
#[test_case("<!DOCTYPE html PUBL\u{130}C \"x\">" ; "dotted capital i in public")]
#[test_case("<!DOCTYPE html \u{212A}YSTEM \"x\">" ; "kelvin sign in system")]
fn non_ascii_keyword_is_not_matched(html: &str) {
    assert_eq!(doctype(html), (s("html"), true, None, None));
    assert_eq!(quirks_mode(html), QuirksMode::Quirks);
}

#[test_case("<!DOCTYPE html sYsTeM \"x\">" ; "mixed case system")]
#[test_case("<!DOCTYPE html system \"x\">" ; "lower case system")]
#[test_case("<!DocType html SYSTEM \"x\">" ; "mixed case doctype")]
fn ascii_system_keyword_matches(html: &str) {
    assert_eq!(doctype(html), (s("html"), false, None, s("x")));
    assert_eq!(quirks_mode(html), QuirksMode::NoQuirks);
}

#[test_case("<!DOCTYPE html pUbLiC \"x\">" ; "mixed case public")]
#[test_case("<!doctype html public \"x\">" ; "lower case")]
fn ascii_public_keyword_matches(html: &str) {
    assert_eq!(doctype(html), (s("html"), false, s("x"), None));
    assert_eq!(quirks_mode(html), QuirksMode::NoQuirks);
}

#[test]
fn ascii_public_keyword_reaches_quirks_tables() {
    let html = "<!DOCTYPE html pUbLiC \"-//W3C//DTD HTML 4.01 Transitional//EN\">";
    assert_eq!(quirks_mode(html), QuirksMode::Quirks);

    let html = "<!DOCTYPE html PuBlIc \"-//W3C//DTD XHTML 1.0 Transitional//EN\" \"x\">";
    assert_eq!(quirks_mode(html), QuirksMode::LimitedQuirks);
}

#[test_case("<!DOCTYPE html>" ; "upper case")]
#[test_case("<!doctype html>" ; "lower case")]
#[test_case("<!DocType html>" ; "mixed case")]
fn ascii_doctype_keyword_matches(html: &str) {
    assert_eq!(doctype(html), (s("html"), false, None, None));
    assert_eq!(quirks_mode(html), QuirksMode::NoQuirks);
}
