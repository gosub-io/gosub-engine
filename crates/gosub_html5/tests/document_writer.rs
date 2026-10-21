//! Escaping and context rules for HTML fragment serialization (#1402).

use std::collections::HashMap;

use gosub_css3::system::Css3System;
use gosub_html5::document::builder::DocumentBuilderImpl;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::node::{HTML_NAMESPACE, MATHML_NAMESPACE, SVG_NAMESPACE};
use gosub_html5::parser::{Html5Parser, Html5ParserOptions};
use gosub_interface::config::ModuleConfiguration;
use gosub_interface::document::Document;
use gosub_shared::byte_stream::{ByteStream, Encoding, Location};
use gosub_shared::node::NodeId;

#[derive(Clone, Debug, PartialEq)]
struct Config;

impl ModuleConfiguration for Config {
    type CssSystem = Css3System;
    type Document = DocumentImpl<Self>;
    type HtmlParser = Html5Parser<'static, Self>;
}

/// Build a subtree directly so parser repair cannot hide a serializer error.
fn element_with_text(name: &str, namespace: Option<&str>, text: &str) -> (DocumentImpl<Config>, NodeId, NodeId) {
    let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
    let element = doc.create_element(name, namespace, HashMap::new(), Location::default());
    let child = doc.create_text(text, Location::default());
    doc.attach(element, doc.root(), None);
    doc.attach(child, element, None);
    (doc, element, child)
}

/// Text quotes stay literal, while attributes escape quotes and both contexts escape markup.
#[test]
fn text_and_attributes_use_their_respective_escape_modes() {
    let value = "&<>\u{a0}\"'é &amp;";
    let (mut doc, element, text) = element_with_text("p", Some(HTML_NAMESPACE), value);
    doc.set_attribute(element, "data-value", value);

    assert_eq!(
        doc.write_from_node(element),
        "<p data-value=\"&amp;&lt;&gt;&nbsp;&quot;'é &amp;amp;\">&amp;&lt;&gt;&nbsp;\"'é &amp;amp;</p>"
    );
    assert_eq!(doc.write_from_node(text), "&amp;&lt;&gt;&nbsp;\"'é &amp;amp;");
    doc.detach(text);
    assert_eq!(doc.write_from_node(text), "&amp;&lt;&gt;&nbsp;\"'é &amp;amp;");
}

/// Data resembling an injected element or attribute remains data after a serialize/parse cycle.
#[test]
fn escaped_values_reparse_as_data_instead_of_markup_or_extra_attributes() {
    let value = "\"><img src=x onerror=alert(1)>&amp;\u{a0}";
    let (mut doc, element, _) = element_with_text("p", Some(HTML_NAMESPACE), value);
    doc.set_attribute(element, "data-value", value);
    let written = doc.write_from_node(element);
    let mut stream = ByteStream::from_str(&written, Encoding::UTF8);
    let mut reparsed = DocumentBuilderImpl::new_document::<Config>(None);
    assert!(Html5Parser::<Config>::parse_document(&mut stream, &mut reparsed, None).is_ok());
    // html -> body -> p. The quoted attribute and the entire text must survive intact.
    let html = reparsed.children(reparsed.root())[0];
    let body = reparsed.children(html)[1];
    let paragraph = reparsed.children(body)[0];
    assert_eq!(reparsed.tag_name(paragraph), Some("p"));
    assert_eq!(reparsed.attributes(paragraph).map(HashMap::len), Some(1));
    assert_eq!(reparsed.attribute(paragraph, "data-value"), Some(value));
    assert_eq!(reparsed.children(paragraph).len(), 1);
    assert_eq!(reparsed.text_value(reparsed.children(paragraph)[0]), Some(value));
}

/// Raw-text context comes from the actual HTML parent, even when only its text node is written.
#[test]
fn html_raw_text_is_literal_even_when_serializing_the_text_node_directly() {
    let value = "if (a < b && b > c) \"&nbsp;\u{a0}\"";
    for name in ["style", "script", "xmp", "iframe", "noembed", "noframes", "plaintext"] {
        let (mut doc, element, text) = element_with_text(name, Some(HTML_NAMESPACE), value);
        // Scripting only controls noscript, not any of these elements.
        doc.set_scripting_enabled(false);
        assert_eq!(doc.write_from_node(element), format!("<{name}>{value}</{name}>"));
        assert_eq!(doc.write_from_node(text), value, "{name}");
    }
}

/// RCDATA is escaped on output so reparsing resolves character references back to the same text.
#[test]
fn rcdata_elements_still_escape_their_text() {
    for name in ["textarea", "title"] {
        let (doc, element, _) = element_with_text(name, Some(HTML_NAMESPACE), "&<tag>\u{a0}");
        assert_eq!(
            doc.write_from_node(element),
            format!("<{name}>&amp;&lt;tag&gt;&nbsp;</{name}>")
        );
    }
}

/// Both scripting modes round-trip noscript source despite producing different text in the DOM.
#[test]
fn noscript_serialization_retains_the_parser_scripting_flag() -> Result<(), Box<dyn std::error::Error>> {
    for scripting_enabled in [false, true] {
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let mut stream = ByteStream::from_str("<body><noscript id=n>&lt;b&gt;&amp;</noscript>", Encoding::UTF8);
        let options = Html5ParserOptions {
            scripting_enabled,
            ..Default::default()
        };
        assert!(Html5Parser::<Config>::parse_document(&mut stream, &mut doc, Some(options)).is_ok());
        assert_eq!(doc.scripting_enabled(), scripting_enabled);
        let noscript = doc.node_by_named_id("n").ok_or("noscript was not parsed")?;
        // The source round-trips in either mode: with scripting enabled it was parsed as raw
        // text; disabled parsing resolved the entities, and serialization must escape them.
        assert_eq!(
            doc.write_from_node(noscript),
            "<noscript id=\"n\">&lt;b&gt;&amp;</noscript>"
        );
    }
    Ok(())
}

/// A later DOM edit still serializes noscript according to the document's scripting state.
#[test]
fn noscript_uses_the_document_flag_for_programmatically_created_text() {
    let (mut doc, element, _) = element_with_text("noscript", Some(HTML_NAMESPACE), "&<b>\u{a0}");
    assert_eq!(doc.write_from_node(element), "<noscript>&<b>\u{a0}</noscript>");
    doc.set_scripting_enabled(false);
    assert_eq!(
        doc.write_from_node(element),
        "<noscript>&amp;&lt;b&gt;&nbsp;</noscript>"
    );
}

/// Void serialization also omits children attached through DOM operations, including obsolete tags.
#[test]
fn html_void_elements_have_neither_end_tags_nor_serialized_children() {
    for name in [
        "area", "base", "basefont", "bgsound", "br", "col", "embed", "frame", "hr", "img", "input", "keygen", "link",
        "meta", "param", "source", "track", "wbr",
    ] {
        let (doc, element, _) = element_with_text(name, Some(HTML_NAMESPACE), "must not be serialized");
        assert_eq!(doc.write_from_node(element), format!("<{name}>"));
    }
}

/// Matching an HTML tag name alone must not alter serialization of elements in other namespaces.
#[test]
fn foreign_names_do_not_get_html_void_or_raw_text_rules() {
    // DocumentImpl treats a missing namespace as HTML, so include an unrelated namespace
    // explicitly as well as SVG and MathML.
    for namespace in [Some(SVG_NAMESPACE), Some(MATHML_NAMESPACE), Some("urn:foreign")] {
        for name in ["br", "img", "frame", "style", "script", "noscript", "plaintext"] {
            let (doc, element, _) = element_with_text(name, namespace, "&<tag>\u{a0}");
            assert_eq!(
                doc.write_from_node(element),
                format!("<{name}>&amp;&lt;tag&gt;&nbsp;</{name}>"),
                "namespace {namespace:?}"
            );
        }
    }
}

/// Restoring an adjusted foreign attribute's prefix does not bypass attribute-value escaping.
#[test]
fn foreign_attributes_keep_their_prefixes_and_escape_their_values() {
    let (mut doc, element, _) = element_with_text("svg", Some(SVG_NAMESPACE), "&<text>");
    doc.set_attribute(element, "xlink href", "?a=1&b=\"<>\u{a0}");
    assert_eq!(
        doc.write_from_node(element),
        "<svg xlink:href=\"?a=1&amp;b=&quot;&lt;&gt;&nbsp;\">&amp;&lt;text&gt;</svg>"
    );
}

/// XML output distinguishes literal entity spellings from NBSP and preserves normalized whitespace.
#[test]
fn xml_mode_escapes_raw_text_without_turning_literal_entities_into_characters() {
    let (mut doc, element, _) = element_with_text("script", Some(HTML_NAMESPACE), "&nbsp; &<>\u{a0}\r");
    doc.set_attribute(element, "data-value", "\"&<>\u{a0}\t\n\r");
    assert_eq!(
        doc.write_xml_from_node(element),
        "<script data-value=\"&quot;&amp;&lt;&gt;\u{a0}&#9;&#10;&#13;\">&amp;nbsp; &amp;&lt;&gt;\u{a0}&#13;</script>"
    );
}

/// XML consumers require balanced tags even for HTML void elements inside foreignObject content.
#[test]
fn xml_mode_closes_html_void_elements_and_keeps_their_children() {
    let (doc, element, _) = element_with_text("br", Some(HTML_NAMESPACE), "&<tag>");
    assert_eq!(doc.write_xml_from_node(element), "<br>&amp;&lt;tag&gt;</br>");
    assert_eq!(doc.write_from_node(element), "<br>");
}
