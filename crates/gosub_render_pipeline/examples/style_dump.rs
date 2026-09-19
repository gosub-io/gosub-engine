//! Dump every element's style, so a change to the style system can be proved to change nothing.
//!
//! The cascade has a lot of moving parts and very little of it is observable from the outside:
//! a refactor that keeps the tests green can still quietly drop a declaration on a page no test
//! covers. This writes down everything the cascade decided - every declaration that reached each
//! property with its cascade facts, and the cascaded, specified, computed, used, actual and
//! inherited value it settled on - for every element of a set of real pages. Run it before a
//! change and after, and diff the two directories.
//!
//! ```sh
//! cargo run -p gosub_render_pipeline --example style_dump -- /tmp/style/before
//! # ... make the change ...
//! cargo run -p gosub_render_pipeline --example style_dump -- /tmp/style/after
//! diff -r /tmp/style/before /tmp/style/after
//! ```
//!
//! Two files per fixture: `<name>.computed.txt` has the resolved values, `<name>.declared.txt`
//! the declarations behind them. Both are keyed by property *name* even where the engine keys
//! itself by something else, and every list is sorted, so the output is comparable across a
//! change to the engine's own keys and stable from run to run.
//!
//! The fixtures are the benchmark's three pages plus the page fixtures in `tests/data`: the
//! wikipedia and stackoverflow DOMs with and without the 2.2 MB real-world sheet, the table
//! layout fixtures, and the navbar pages.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fmt::Write as _;

use gosub_css3::matcher::styling::CssProperties;
use gosub_css3::stylesheet::{set_layout_viewport, CssStylesheet, CssValue};
use gosub_css3::system::Css3System;
use gosub_css3::Css3;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::html_compile;
use gosub_html5::parser::Html5Parser;
use gosub_interface::config::ModuleConfiguration;
use gosub_interface::css3::{CssOrigin, CssPropertyMap, CssSystem as _};
use gosub_interface::document::Document as _;
use gosub_interface::node::NodeType;
use gosub_shared::config::ParserConfig;
use gosub_shared::node::NodeId;

// The same fixture pages the style benchmark measures, so the two describe one thing.
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

/// Pinned, because `@media` conditions and viewport units are part of what is being compared.
const VIEWPORT: (f32, f32) = (1280.0, 800.0);

const DATA_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/data");

struct Fixture {
    name: String,
    doc: DocumentImpl<Config>,
}

fn parse_author_sheet(css: &str, name: &str) -> CssStylesheet {
    let config = ParserConfig {
        ignore_errors: true,
        ..Default::default()
    };
    Css3::parse_str(css, config, CssOrigin::Author, name).expect("fixture stylesheet should parse")
}

fn build_fixture(name: &str, html: &str, author_css: &[(&str, &str)]) -> Fixture {
    let mut doc = html_compile::<Config>(html);
    doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());
    for (sheet_name, css) in author_css {
        doc.add_stylesheet(parse_author_sheet(css, sheet_name));
    }
    Fixture {
        name: name.to_string(),
        doc,
    }
}

fn file_fixture(name: &str, path: &str, author_css: &[(&str, &str)]) -> Fixture {
    let html = std::fs::read_to_string(format!("{DATA_DIR}/{path}"))
        .unwrap_or_else(|e| panic!("fixture {path} should exist: {e}"));
    build_fixture(name, &html, author_css)
}

fn fixtures() -> Vec<Fixture> {
    let real_world_css =
        std::fs::read_to_string(format!("{DATA_DIR}/css3-data/data.css")).expect("tests/data/css3-data/data.css");
    let utility = generate_utility_page();

    let mut fixtures = vec![
        build_fixture("small", SMALL_HTML, &[]),
        build_fixture("utility-3k", &utility.html, &[("utility.css", &utility.css)]),
        file_fixture("wikipedia", "tree_iterator/wikipedia_main.html", &[]),
        file_fixture(
            "wikipedia+2.2m",
            "tree_iterator/wikipedia_main.html",
            &[("data.css", &real_world_css)],
        ),
        file_fixture("stackoverflow", "tree_iterator/stackoverflow.html", &[]),
        file_fixture(
            "stackoverflow+2.2m",
            "tree_iterator/stackoverflow.html",
            &[("data.css", &real_world_css)],
        ),
        file_fixture("fixed-navbar", "fixed-navbar.html", &[]),
        file_fixture("opacity-navbar", "opacity-navbar.html", &[]),
        file_fixture("sticky-navbar", "sticky-navbar.html", &[]),
    ];

    // Every table fixture, in name order so the run is the same on every machine.
    let mut tables: Vec<String> = std::fs::read_dir(format!("{DATA_DIR}/tables"))
        .expect("tests/data/tables")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".html"))
        .collect();
    tables.sort();
    for name in tables {
        fixtures.push(file_fixture(&format!("tables_{name}"), &format!("tables/{name}"), &[]));
    }

    fixtures
}

/// Style one element and write down everything the cascade decided about it.
fn dump_element(
    doc: &DocumentImpl<Config>,
    id: NodeId,
    map: &mut CssProperties,
    computed: &mut String,
    declared: &mut String,
) {
    let tag = doc.tag_name(id).unwrap_or_default();
    let _ = writeln!(computed, "#{} <{tag}>", usize::from(id));
    let _ = writeln!(declared, "#{} <{tag}>", usize::from(id));

    // Resolving is what fills in cascaded/specified/computed/used/actual, so it happens before
    // anything is read - exactly as the render pipeline's adapter does it.
    for (_, property) in map.iter_mut() {
        property.compute_value();
    }

    let mut names: Vec<String> = map.iter().map(|(name, _)| name.to_string()).collect();
    names.sort();

    for name in &names {
        let Some(property) = <CssProperties as CssPropertyMap<Css3System>>::get(map, name) else {
            continue;
        };
        let _ = writeln!(
            computed,
            "  {name} = {:?} | cascaded {:?} | specified {:?} | computed {:?} | used {:?} | inherited {:?} | em {} rem {}",
            property.actual,
            property.cascaded,
            property.specified,
            property.computed,
            property.used,
            property.inherited,
            property.font_size_basis,
            property.root_font_size_basis
        );
        for declaration in &property.declared {
            let _ = writeln!(
                declared,
                "  {name} << {:?} origin={:?} imp={} spec={:?} depth={} order={} layer={:?} attached={} loc={}",
                declaration.value,
                declaration.origin,
                declaration.important,
                declaration.specificity,
                declaration.shadow_depth,
                declaration.order,
                declaration.layer,
                declaration.attached,
                declaration.location
            );
        }
    }

    // The custom-property scope in force on this element, which is what a `var()` below it reads.
    let mut custom: Vec<(&String, &CssValue)> = map.custom.iter().collect();
    custom.sort_by(|a, b| a.0.cmp(b.0));
    for (name, value) in custom {
        let _ = writeln!(computed, "  custom {name} = {value:?}");
    }
    let _ = writeln!(
        computed,
        "  font_size_px={} root_font_size_px={}",
        map.font_size_px, map.root_font_size_px
    );
}

fn dump_subtree(
    doc: &DocumentImpl<Config>,
    id: NodeId,
    sheets: &[CssStylesheet],
    parent: Option<&CssProperties>,
    computed: &mut String,
    declared: &mut String,
) {
    let own = if doc.node_type(id) == NodeType::ElementNode {
        let mut map = Css3System::properties_from_node::<Config>(doc, id, sheets, parent);
        if let Some(map) = map.as_mut() {
            dump_element(doc, id, map, computed, declared);
        }
        map
    } else {
        None
    };

    // A text node takes no style of its own, so its element ancestor is what its siblings and
    // children inherit from - the same flat-parent rule the render pipeline's adapter uses.
    let inherited = own.as_ref().or(parent);
    for &child in doc.children(id) {
        dump_subtree(doc, child, sheets, inherited, computed, declared);
    }
}

fn main() {
    let Some(out_dir) = std::env::args().nth(1) else {
        eprintln!("usage: cargo run -p gosub_render_pipeline --example style_dump -- <out-dir>");
        std::process::exit(2);
    };
    std::fs::create_dir_all(&out_dir).expect("output directory");

    for fixture in fixtures() {
        // Set per fixture as well as once up front: building a fixture parses stylesheets, and
        // the viewport is global state the parse reads.
        set_layout_viewport(VIEWPORT.0, VIEWPORT.1);

        let mut computed = String::new();
        let mut declared = String::new();
        let root = fixture.doc.root();
        dump_subtree(
            &fixture.doc,
            root,
            fixture.doc.stylesheets(),
            None,
            &mut computed,
            &mut declared,
        );

        std::fs::write(format!("{out_dir}/{}.computed.txt", fixture.name), computed).expect("write");
        std::fs::write(format!("{out_dir}/{}.declared.txt", fixture.name), declared).expect("write");
        eprintln!("dumped {}", fixture.name);
    }
}
