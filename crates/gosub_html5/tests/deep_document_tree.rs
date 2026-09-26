//! The tree builder stops nesting past 512 levels, as Blink does, and places deeper elements
//! beside their parent instead. Everything after the parser - style, the render tree, layout,
//! dropping the document - does work per level, so a page nesting 20,000 elements aborted the
//! process on the stack. Follow-up to GHSA-c762-mxfh-vwvp.

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

/// `MAX_TREE_DEPTH` in the tree builder.
const CAP: usize = 512;

fn parse(html: &str) -> DocumentImpl<Config> {
    let mut stream = ByteStream::from_str(html, Encoding::UTF8);
    let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
    let _ = Html5Parser::<Config>::parse_document(&mut stream, &mut doc, None);
    doc
}

/// The deepest element below the document, and how many elements are called `tag`. Iterative, so
/// it can measure a tree the cap failed to limit.
fn depth_and_count(doc: &DocumentImpl<Config>, tag: &str) -> (usize, usize) {
    let mut deepest = 0;
    let mut count = 0;
    let mut pending = vec![(NodeId::root(), 0)];
    while let Some((id, depth)) = pending.pop() {
        deepest = deepest.max(depth);
        if doc.tag_name(id) == Some(tag) {
            count += 1;
        }
        pending.extend(doc.children(id).iter().map(|&child| (child, depth + 1)));
    }
    (deepest, count)
}

fn nested(open: &str, close: &str, depth: usize) -> String {
    format!(
        "<html><body>{}x{}</body></html>",
        open.repeat(depth),
        close.repeat(depth)
    )
}

#[test]
fn deep_nesting_is_capped_without_losing_elements() {
    // `<div>` and `<ul>` cost the tree builder a scan of the open elements per start tag, so they
    // get fewer levels; 5000 is still ten times the cap.
    let cases = [
        ("div", nested("<div>", "</div>", 5_000), 5_000),
        ("span", nested("<span>", "</span>", 20_000), 20_000),
        ("b", nested("<b>", "</b>", 20_000), 20_000),
        ("b", format!("<html><body>{}x", "<b>".repeat(20_000)), 20_000),
        ("td", nested("<table><tr><td>", "</td></tr></table>", 20_000), 20_000),
        ("template", nested("<template>", "</template>", 20_000), 20_000),
        (
            "g",
            format!(
                "<html><body><svg>{}<rect/>{}</svg></body></html>",
                "<g>".repeat(20_000),
                "</g>".repeat(20_000)
            ),
            20_000,
        ),
    ];

    for (tag, html, levels) in cases {
        let doc = parse(&html);
        let (depth, count) = depth_and_count(&doc, tag);
        // A few levels of slack for `html`, `body`, and the wrappers around each case.
        assert!(
            (CAP..=CAP + 5).contains(&depth),
            "<{tag}> x {levels}: tree is {depth} deep, expected the cap of {CAP}"
        );
        // Placed beside their parent, not dropped.
        assert_eq!(count, levels, "<{tag}> x {levels}: elements went missing");
    }
}

#[test]
fn nesting_below_the_cap_is_untouched() {
    let doc = parse(&nested("<div>", "</div>", CAP - 10));
    let (depth, count) = depth_and_count(&doc, "div");
    assert_eq!(count, CAP - 10);
    // document > html > body > divs > the text
    assert_eq!(depth, CAP - 10 + 3);
}
