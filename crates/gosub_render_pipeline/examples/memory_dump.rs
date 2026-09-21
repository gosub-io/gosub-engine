//! Where a page's memory goes, for the same fixtures the style benchmark measures.
//!
//! ```sh
//! cargo run --release -p gosub_render_pipeline --example memory_dump
//! cargo run --release -p gosub_render_pipeline --example memory_dump -- wikipedia+2.2m
//! ```
//!
//! The snapshot is taken after every element has been styled and converted, which is the moment
//! worth comparing: the document is parsed, the cascade has run, and every element holds the
//! typed struct layout and paint read. Shared allocations are counted once - see
//! [`gosub_shared::memory`] for why that is the only honest way to report this engine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use gosub_css3::matcher::computed_style::computed_style;
use gosub_css3::matcher::styling::CssProperties;
use gosub_css3::stylesheet::{set_layout_viewport, CssStylesheet};
use gosub_css3::system::Css3System;
use gosub_css3::Css3;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::html_compile;
use gosub_html5::parser::Html5Parser;
use gosub_interface::config::ModuleConfiguration;
use gosub_interface::css3::{CssOrigin, CssPropertyMap as _, CssSystem as _};
use gosub_interface::document::Document as _;
use gosub_interface::node::NodeType;
use gosub_interface::style::ComputedStyle;
use gosub_shared::config::ParserConfig;
use gosub_shared::memory::{record, HeapSize, Row, Walk};
use gosub_shared::node::NodeId;

#[path = "../benches/common/pages.rs"]
mod pages;

use pages::{generate_utility_page, SMALL_HTML};

#[derive(Clone, Debug, PartialEq)]
struct Config;

impl ModuleConfiguration for Config {
    type CssSystem = Css3System;
    type Document = DocumentImpl<Self>;
    type HtmlParser = Html5Parser<'static, Self>;
}

const VIEWPORT: (f32, f32) = (1280.0, 800.0);

/// A fixture, built on demand so only the one being reported is ever in memory.
type Fixture = (&'static str, Box<dyn FnOnce() -> DocumentImpl<Config>>);

fn parse_author_sheet(css: &str, name: &str) -> CssStylesheet {
    let config = ParserConfig {
        ignore_errors: true,
        ..Default::default()
    };
    Css3::parse_str(css, config, CssOrigin::Author, name).expect("stylesheet should parse")
}

fn build(html: &str, author_css: &[(&str, &str)]) -> DocumentImpl<Config> {
    set_layout_viewport(VIEWPORT.0, VIEWPORT.1);
    let mut doc = html_compile::<Config>(html);
    doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());
    for (sheet_name, css) in author_css {
        doc.add_stylesheet(parse_author_sheet(css, sheet_name));
    }
    doc
}

/// Style every element top-down, keeping every map and every struct, which is what the pipeline
/// holds while a page is on screen.
fn style_everything(doc: &DocumentImpl<Config>) -> (Vec<CssProperties>, Vec<ComputedStyle>) {
    let mut maps = Vec::new();
    let mut styles = Vec::new();
    walk(doc, doc.root(), None, None, &mut maps, &mut styles);
    (maps, styles)
}

fn walk(
    doc: &DocumentImpl<Config>,
    id: NodeId,
    parent_map: Option<&CssProperties>,
    parent_style: Option<&ComputedStyle>,
    maps: &mut Vec<CssProperties>,
    styles: &mut Vec<ComputedStyle>,
) {
    let own = if doc.node_type(id) == NodeType::ElementNode {
        let mut map = Css3System::properties_from_node::<Config>(doc, id, doc.stylesheets(), parent_map);
        if let Some(map) = map.as_mut() {
            for (_, property) in map.iter_mut() {
                property.compute_value();
            }
        }
        map
    } else {
        None
    };

    let style = own.as_ref().map(|map| computed_style(map, parent_style));

    let inherited_map = own.as_ref().or(parent_map);
    let inherited_style = style.as_ref().or(parent_style);
    // The children are walked while this element's map and struct are still borrowed, so the
    // recursion happens before either is moved into the collections below.
    let children: Vec<NodeId> = doc.children(id).to_vec();
    for child in children {
        walk(doc, child, inherited_map, inherited_style, maps, styles);
    }

    if let Some(map) = own {
        maps.push(map);
    }
    if let Some(style) = style {
        styles.push(style);
    }
}

fn record_computed_styles(styles: &[ComputedStyle], walk: &mut Walk) {
    for style in styles {
        style.heap_size(walk);
    }
    let (owned, shared) = walk.take_counts();
    record(
        Row::new(
            "css.computed_style",
            styles.len() as u64,
            size_of_val(styles),
            owned,
            shared,
        )
        .with_note("eleven Arc groups per element; an untouched group is the parent's or the initial one"),
    );
}

fn report(name: &str, doc: DocumentImpl<Config>) {
    let (maps, styles) = style_everything(&doc);

    gosub_shared::memory::begin(format!("{name}, after styling every element"));
    let mut walk = Walk::new();

    gosub_html5::memory::record_document(&doc, &mut walk);
    gosub_css3::memory::record_property_maps(|| maps.iter(), &mut walk);
    record_computed_styles(&styles, &mut walk);

    gosub_shared::memory::dump();
}

fn main() {
    let wanted = std::env::args().nth(1);
    let data_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/data");

    let fixtures: Vec<Fixture> = vec![
        ("small", Box::new(|| build(SMALL_HTML, &[]))),
        (
            "utility-3k",
            Box::new(|| {
                let utility = generate_utility_page();
                build(&utility.html, &[("utility.css", &utility.css)])
            }),
        ),
        (
            "wikipedia+2.2m",
            Box::new(move || {
                let html = std::fs::read_to_string(format!("{data_dir}/tree_iterator/wikipedia_main.html"))
                    .expect("wikipedia fixture should exist");
                let css = std::fs::read_to_string(format!("{data_dir}/css3-data/data.css"))
                    .expect("the 2.2 MB sheet should exist");
                build(&html, &[("data.css", &css)])
            }),
        ),
    ];

    for (name, make) in fixtures {
        if let Some(wanted) = &wanted {
            if wanted != name {
                continue;
            }
        }
        report(name, make());
    }
}
