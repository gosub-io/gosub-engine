/// Fetch a URL, parse it through gosub's HTML5+CSS3 pipeline, build a
/// RenderTree, and print it to stdout.
///
/// Usage: cargo run --example rendertree -p gosub_pipeline -- <url>
use std::sync::Arc;

use gosub_css3::system::Css3System;
use gosub_html5::document::builder::DocumentBuilderImpl;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::parser::Html5Parser;
use gosub_interface::config::{HasCssSystem, HasDocument};
use gosub_interface::css3::CssSystem as _;
use gosub_interface::document::Document as _;
use gosub_shared::byte_stream::{ByteStream, Encoding};
use url::Url;

use cow_utils::CowUtils;
use gosub_pipeline::common::document::node::NodeType;
use gosub_pipeline::common::document::pipeline_doc::GosubDocumentAdapter;
use gosub_pipeline::common::document::style::{StyleProperty, StyleValue, Unit};
use gosub_pipeline::rendertree_builder::{RenderNodeId, RenderTree};

// ---- Engine config wiring gosub_html5 + gosub_css3 ----

#[derive(Clone, Debug, PartialEq)]
struct Config;

impl HasCssSystem for Config {
    type CssSystem = Css3System;
}
impl HasDocument for Config {
    type Document = DocumentImpl<Self>;
}

// -------------------------------------------------------

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: rendertree <url>");
        std::process::exit(1);
    }

    let url_str = &args[1];
    let url = Url::parse(url_str).unwrap_or_else(|e| {
        eprintln!("Invalid URL '{url_str}': {e}");
        std::process::exit(1);
    });

    eprintln!("Fetching {url}…");
    let html = fetch_html(url_str).unwrap_or_else(|e| {
        eprintln!("Fetch failed: {e}");
        std::process::exit(1);
    });
    eprintln!("  {} bytes received", html.len());

    eprintln!("Parsing HTML…");
    let mut doc = DocumentBuilderImpl::new_document::<Config>(Some(url.clone()));
    let mut stream = ByteStream::from_str(&html, Encoding::UTF8);
    let _ = Html5Parser::<Config>::parse_document(&mut stream, &mut doc, None);

    let ua = Css3System::load_default_useragent_stylesheet();
    doc.add_stylesheet(ua);
    eprintln!("  {} stylesheet(s) loaded", doc.stylesheets().len());

    eprintln!("Building render tree…");
    let adapter = GosubDocumentAdapter::<Config>::new(doc);
    let mut rt = RenderTree::new(Arc::new(adapter));
    rt.parse();

    println!();
    println!("Render tree for: {url}");
    println!("  nodes in tree : {}", rt.count_elements());
    println!("{}", "─".repeat(72));

    if let Some(root_id) = rt.root_id {
        print_subtree(&rt, root_id, 0);
    } else {
        println!("(empty render tree)");
    }
}

fn fetch_html(url: &str) -> anyhow::Result<String> {
    let parsed = url::Url::parse(url)?;
    let response = gosub_net::http::blocking::get(&parsed)?;
    Ok(String::from_utf8_lossy(&response.body).into_owned())
}

fn print_subtree(rt: &RenderTree, id: RenderNodeId, depth: usize) {
    let Some(render_node) = rt.get_node_by_id(id) else {
        return;
    };

    if let Some(line) = node_label(rt, id, depth) {
        println!("{line}");
    }

    for &child_id in &render_node.children {
        print_subtree(rt, child_id, depth + 1);
    }
}

fn node_label(rt: &RenderTree, id: RenderNodeId, depth: usize) -> Option<String> {
    let node = rt.get_document_node_by_render_id(id)?;
    let indent = "  ".repeat(depth);

    let content = match &node.node_type {
        NodeType::Comment(_) => return None,

        NodeType::Text(text, _) => {
            let t = text.trim();
            if t.is_empty() {
                return None;
            }
            let preview: String = t.chars().take(72).collect();
            let ellipsis = if t.chars().count() > 72 { "…" } else { "" };
            format!("{indent}\"{preview}{ellipsis}\"")
        }

        NodeType::Element(data) => {
            // Tag + selected CSS properties
            let mut parts: Vec<String> = vec![format!("<{}>", data.tag_name)];

            for (label, prop) in [
                ("display", StyleProperty::Display),
                ("w", StyleProperty::Width),
                ("h", StyleProperty::Height),
                ("color", StyleProperty::Color),
                ("bg", StyleProperty::BackgroundColor),
            ] {
                if let Some(v) = data.styles.get_property(prop) {
                    parts.push(format!("{}={}", label, fmt_value(v)));
                }
            }

            // id / class attributes (useful for orientation)
            if let Some(id_attr) = data.attributes.get("id") {
                parts.push(format!("#{id_attr}"));
            }
            if let Some(cls) = data.attributes.get("class") {
                let abbreviated: String = cls.split_whitespace().take(2).collect::<Vec<_>>().join(" ");
                parts.push(format!(".{abbreviated}"));
            }

            format!("{indent}{}", parts.join("  "))
        }
    };

    Some(content)
}

fn fmt_value(v: &StyleValue) -> String {
    match v {
        StyleValue::Unit(n, Unit::Px) => format!("{n}px"),
        StyleValue::Unit(n, Unit::Percent) => format!("{n}%"),
        StyleValue::Unit(n, Unit::Em) => format!("{n}em"),
        StyleValue::Unit(n, Unit::Rem) => format!("{n}rem"),
        StyleValue::Number(n) => format!("{n}"),
        StyleValue::Keyword(k) => k.clone(),
        StyleValue::Display(d) => format!("{d:?}").cow_to_ascii_lowercase().into_owned(),
        StyleValue::Color(c) => format!("{c:?}"),
        _ => format!("{v:?}"),
    }
}
