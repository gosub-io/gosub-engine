//! `Document::clone_node`: a deep copy of a node and everything below it, left unattached.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashSet;

use gosub_css3::system::Css3System;
use gosub_html5::document::builder::DocumentBuilderImpl;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::parser::Html5Parser;
use gosub_interface::config::{HasDocument, ModuleConfiguration};
use gosub_interface::document::Document;
use gosub_shared::attributes::Attributes;
use gosub_shared::byte_stream::{ByteStream, Encoding, Location};
use gosub_shared::node::NodeId;

#[derive(Clone, Debug, PartialEq)]
struct Config;

impl ModuleConfiguration for Config {
    type CssSystem = Css3System;
    type Document = DocumentImpl<Self>;
    type HtmlParser = Html5Parser<'static, Self>;
}

fn parse(html: &str) -> DocumentImpl<Config> {
    let mut stream = ByteStream::from_str(html, Encoding::UTF8);
    let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
    let _ = Html5Parser::<Config>::parse_document(&mut stream, &mut doc, None);
    doc
}

/// Depth-first search for the first element with the given tag name.
fn find<C: HasDocument>(doc: &C::Document, tag: &str) -> Option<NodeId> {
    fn walk<C: HasDocument>(doc: &C::Document, id: NodeId, tag: &str) -> Option<NodeId> {
        if doc.tag_name(id) == Some(tag) {
            return Some(id);
        }
        doc.children(id)
            .to_vec()
            .into_iter()
            .find_map(|c| walk::<C>(doc, c, tag))
    }
    walk::<C>(doc, doc.root(), tag)
}

/// `root` and every node below it.
fn subtree(doc: &DocumentImpl<Config>, root: NodeId) -> Vec<NodeId> {
    let mut nodes = Vec::new();
    let mut pending = vec![root];
    while let Some(id) = pending.pop() {
        nodes.push(id);
        pending.extend(doc.children(id).iter().copied());
    }
    nodes
}

#[test]
fn a_clone_copies_the_whole_subtree() {
    let mut doc =
        parse(r#"<body><div id=x class=a><p>one<b>two</b></p><!--c--><span title=t>three</span></div></body>"#);
    let body = find::<Config>(&doc, "body").unwrap();

    let clone = doc.clone_node(body);

    assert_ne!(clone, body);
    assert_eq!(doc.write_from_node(clone), doc.write_from_node(body));
    assert_eq!(subtree(&doc, clone).len(), subtree(&doc, body).len());
}

#[test]
fn a_clone_is_unattached_and_shares_no_node_with_the_original() {
    let mut doc = parse("<body><div><p>one<b>two</b></p><span>three</span></div></body>");
    let div = find::<Config>(&doc, "div").unwrap();
    let before = doc.write_from_node(doc.root());

    let clone = doc.clone_node(div);

    assert_eq!(doc.parent(clone), None, "the clone is not inserted anywhere");
    assert_eq!(doc.write_from_node(doc.root()), before, "the document is untouched");

    let original: HashSet<NodeId> = subtree(&doc, div).into_iter().collect();
    let copied: HashSet<NodeId> = subtree(&doc, clone).into_iter().collect();
    assert_eq!(copied.len(), original.len());
    assert!(original.is_disjoint(&copied));
    for &id in &copied {
        if id != clone {
            assert!(copied.contains(&doc.parent(id).unwrap()), "{id} hangs below the clone");
        }
    }
}

#[test]
fn changing_a_clone_leaves_the_original_alone() {
    let mut doc = parse("<body><div><p title=old>text</p></div></body>");
    let div = find::<Config>(&doc, "div").unwrap();
    let p = find::<Config>(&doc, "p").unwrap();

    let clone = doc.clone_node(div);
    let cloned_p = doc.children(clone)[0];
    doc.set_attribute(cloned_p, "title", "new");
    let text = doc.children(cloned_p)[0];
    doc.remove(text);

    assert_eq!(doc.attribute(p, "title"), Some("old"));
    assert_eq!(doc.write_from_node(div), r#"<div><p title="old">text</p></div>"#);
}

#[test]
fn a_leaf_clones_to_a_copy_of_itself() {
    let mut doc = parse("<body><p>text<!--note--><br></p></body>");
    let p = find::<Config>(&doc, "p").unwrap();

    for leaf in doc.children(p).to_vec() {
        let clone = doc.clone_node(leaf);
        assert_ne!(clone, leaf);
        assert_eq!(doc.node_type(clone), doc.node_type(leaf));
        assert_eq!(doc.write_from_node(clone), doc.write_from_node(leaf));
        assert!(doc.children(clone).is_empty());
    }
}

#[test]
fn ids_in_a_clone_count_once_it_is_inserted() {
    let mut doc = parse("<body><div><p id=x>original</p></div></body>");
    let body = find::<Config>(&doc, "body").unwrap();
    let div = find::<Config>(&doc, "div").unwrap();
    let p = find::<Config>(&doc, "p").unwrap();

    let clone = doc.clone_node(div);
    let cloned_p = doc.children(clone)[0];
    assert_eq!(doc.node_by_named_id("x"), Some(p), "a detached clone is in no tree");

    doc.attach(clone, body, Some(0)).unwrap();
    assert_eq!(
        doc.node_by_named_id("x"),
        Some(cloned_p),
        "the clone comes first in tree order"
    );
}

#[test]
fn a_cloned_template_holds_its_own_contents() {
    // A template keeps its contents as its own children and points at itself. A copy that kept
    // pointing at the original would send its contents, and any parser insert, there instead.
    let mut doc = parse("<body><template><p id=t>inside</p></template></body>");
    let body = find::<Config>(&doc, "body").unwrap();
    let template = find::<Config>(&doc, "template").unwrap();

    let clone = doc.clone_node(template);

    assert_eq!(doc.template_contents(clone), Some(clone));
    assert_eq!(doc.template_contents(template), Some(template));
    assert_eq!(doc.write_from_node(clone), doc.write_from_node(template));

    doc.attach(clone, body, None).unwrap();
    assert_eq!(
        doc.node_by_named_id("t"),
        None,
        "template contents keep their ids to themselves"
    );
}

#[test]
fn a_template_cloned_inside_a_subtree_holds_its_own_contents() {
    let mut doc = parse("<body><div><template><p id=t>inside</p></template></div></body>");
    let body = find::<Config>(&doc, "body").unwrap();
    let div = find::<Config>(&doc, "div").unwrap();

    let clone = doc.clone_node(div);
    let cloned_template = doc.children(clone)[0];

    assert_eq!(doc.template_contents(cloned_template), Some(cloned_template));
    doc.attach(clone, body, Some(0)).unwrap();
    assert_eq!(doc.node_by_named_id("t"), None);
}

#[test]
fn a_template_whose_contents_live_elsewhere_clones_without_them() {
    // The parser does this for a declarative shadow root: the contents are that root, which
    // belongs to the original template alone.
    let mut doc = parse("<body></body>");
    let template = doc.create_element("template", None, Attributes::new(), Location::default());
    let fragment = doc.create_element("div", None, Attributes::new(), Location::default());
    doc.set_template_contents(template, fragment);

    let clone = doc.clone_node(template);

    assert_eq!(doc.template_contents(clone), None);
    assert_eq!(doc.template_contents(template), Some(fragment));
}

#[test]
fn a_shadow_host_below_the_clone_keeps_its_shadow_tree() {
    let mut doc =
        parse("<section><div><template shadowrootmode=open><span>s</span></template><p>light</p></div></section>");
    let section = find::<Config>(&doc, "section").unwrap();
    let host = find::<Config>(&doc, "div").unwrap();
    let root = doc.shadow_root(host).expect("declarative shadow root");

    let clone = doc.clone_node(section);
    let cloned_host = doc.children(clone)[0];

    assert_eq!(
        doc.shadow_root(cloned_host),
        None,
        "a clone starts without a shadow tree"
    );
    assert_eq!(doc.shadow_root(host), Some(root));
    assert_eq!(doc.shadow_host(root), Some(host));
    assert_eq!(doc.write_from_node(clone), "<section><div><p>light</p></div></section>");

    doc.remove(clone);
    assert_eq!(
        doc.shadow_root(host),
        Some(root),
        "removing the copy takes nothing of the original"
    );
}

#[test]
fn a_very_deep_subtree_clones_without_running_out_of_stack() {
    // The parser caps nesting, but the DOM API does not.
    const DEPTH: usize = 100_000;
    let mut doc = parse("<body></body>");
    let body = find::<Config>(&doc, "body").unwrap();
    let top = doc.create_element("div", None, Attributes::new(), Location::default());
    doc.attach(top, body, None).unwrap();
    let mut parent = top;
    for _ in 1..DEPTH {
        let child = doc.create_element("div", None, Attributes::new(), Location::default());
        doc.attach(child, parent, None).unwrap();
        parent = child;
    }

    let clone = doc.clone_node(top);

    let mut depth = 1;
    let mut current = clone;
    while let Some(&child) = doc.children(current).first() {
        depth += 1;
        current = child;
    }
    assert_eq!(depth, DEPTH);
}
