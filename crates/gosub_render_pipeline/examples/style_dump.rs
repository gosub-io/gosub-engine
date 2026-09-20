//! Dump every element's style, so a change to the style system can be proved to change nothing.
//!
//! The cascade has a lot of moving parts and very little of it is observable from the outside:
//! a refactor that keeps the tests green can still quietly drop a declaration on a page no test
//! covers. This writes down everything the cascade decided - every declaration that reached each
//! property with its cascade facts, and the cascaded, specified, computed and inherited value it
//! settled on - for every element of a set of real pages. Run it before a change and after, and
//! diff the two directories.
//!
//! ```sh
//! cargo run -p gosub_render_pipeline --example style_dump -- /tmp/style/before
//! # ... make the change ...
//! cargo run -p gosub_render_pipeline --example style_dump -- /tmp/style/after
//! diff -r /tmp/style/before /tmp/style/after
//! ```
//!
//! With `--stats` after the directory it also prints, per fixture, how much the maps cost: how
//! many elements got one, how many property entries they hold between them, and how many
//! declarations reached those entries.
//!
//! Two files per fixture: `<name>.computed.txt` has the resolved values, `<name>.declared.txt`
//! the declarations behind them. Both are keyed by property *name* even where the engine keys
//! itself by something else, and every list is sorted, so the output is comparable across a
//! change to the engine's own keys and stable from run to run.
//!
//! The computed dump has a line per property the element's own cascade declared, and then a
//! line per property it inherits a value for. Those are two different questions - what this
//! element said, and what it takes from above - and asking them separately is what makes the
//! dump comparable across a change to *how* inheritance is carried down, which is an engine
//! detail rather than something the cascade decided.
//!
//! The fixtures are the benchmark's three pages plus the page fixtures in `tests/data`: the
//! wikipedia and stackoverflow DOMs with and without the 2.2 MB real-world sheet, the table
//! layout fixtures, and the navbar pages.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fmt::Write as _;

use gosub_css3::matcher::property_ids::{PropertyId, PROPERTY_COUNT};
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

/// What one fixture's property maps cost, for `--stats`.
#[derive(Default)]
struct Stats {
    /// Elements that got a map of their own.
    elements: usize,
    /// Property entries across all of those maps.
    entries: usize,
    /// Declarations recorded on those entries.
    declarations: usize,
}

/// Style one element and write down everything the cascade decided about it.
fn dump_element(
    doc: &DocumentImpl<Config>,
    id: NodeId,
    map: &mut CssProperties,
    computed: &mut String,
    declared: &mut String,
    stats: &mut Stats,
) {
    let tag = doc.tag_name(id).unwrap_or_default();
    let _ = writeln!(computed, "#{} <{tag}>", usize::from(id));
    let _ = writeln!(declared, "#{} <{tag}>", usize::from(id));

    // Resolving is what fills in cascaded/specified/computed, so it happens before anything is
    // read - exactly as the render pipeline's adapter does it.
    for (_, property) in map.iter_mut() {
        property.compute_value();
    }

    stats.elements += 1;
    stats.entries += map.len();
    stats.declarations += map.iter().map(|(_, property)| property.declared.len()).sum::<usize>();

    let mut names: Vec<String> = map.iter().map(|(name, _)| name.to_string()).collect();
    names.sort();

    for name in &names {
        let Some(property) = <CssProperties as CssPropertyMap<Css3System>>::get(map, name) else {
            continue;
        };
        // An entry with no declaration behind it is not something this element's cascade
        // decided - it is inheritance, which the next block reports for every property at once.
        if property.declared.is_empty() {
            continue;
        }
        // The inherited value is asked of the map, not read off the entry: whether an entry
        // carries one is an engine detail, and what the element inherits is not.
        let inherited = PropertyId::from_name(name).and_then(|id| map.inherited_value(id));
        let _ = writeln!(
            computed,
            "  {name} | cascaded {:?} | specified {:?} | computed {:?} | inherited {:?} | em {} rem {}",
            property.cascaded,
            property.specified,
            property.computed,
            inherited.unwrap_or(&CssValue::None),
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

    // What the element takes from above, for every property that has an answer - asked of the
    // map rather than read off an entry, since whether an entry exists is an engine detail.
    for index in 0..PROPERTY_COUNT {
        let Some(id) = PropertyId::from_index(index) else {
            continue;
        };
        // A property that does not inherit still has an inherited value - that is what
        // `inherit` names on a `width` - but only where the element declared it does anything
        // ever ask for it, so only there is it part of what the cascade decided.
        let wanted = id.inherited() || map.get_id(id).is_some_and(|property| !property.declared.is_empty());
        if !wanted {
            continue;
        }
        if let Some(value) = map.inherited_value(id) {
            let _ = writeln!(computed, "  inherits {} = {value:?}", id.name());
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
    stats: &mut Stats,
) {
    let own = if doc.node_type(id) == NodeType::ElementNode {
        let mut map = Css3System::properties_from_node::<Config>(doc, id, sheets, parent);
        if let Some(map) = map.as_mut() {
            dump_element(doc, id, map, computed, declared, stats);
        }
        map
    } else {
        None
    };

    // A text node takes no style of its own, so its element ancestor is what its siblings and
    // children inherit from - the same flat-parent rule the render pipeline's adapter uses.
    let inherited = own.as_ref().or(parent);
    for &child in doc.children(id) {
        dump_subtree(doc, child, sheets, inherited, computed, declared, stats);
    }
}

fn main() {
    let Some(out_dir) = std::env::args().nth(1) else {
        eprintln!("usage: cargo run -p gosub_render_pipeline --example style_dump -- <out-dir> [--stats]");
        std::process::exit(2);
    };
    let want_stats = std::env::args().any(|arg| arg == "--stats");
    std::fs::create_dir_all(&out_dir).expect("output directory");

    for fixture in fixtures() {
        // Set per fixture as well as once up front: building a fixture parses stylesheets, and
        // the viewport is global state the parse reads.
        set_layout_viewport(VIEWPORT.0, VIEWPORT.1);

        let mut computed = String::new();
        let mut declared = String::new();
        let mut stats = Stats::default();
        let root = fixture.doc.root();
        dump_subtree(
            &fixture.doc,
            root,
            fixture.doc.stylesheets(),
            None,
            &mut computed,
            &mut declared,
            &mut stats,
        );

        // Printed rather than written into the output directory, so a `diff -r` of two runs
        // still compares the dumps and nothing else.
        if want_stats {
            println!(
                "{} elements={} entries={} declarations={}",
                fixture.name, stats.elements, stats.entries, stats.declarations
            );
        }

        std::fs::write(format!("{out_dir}/{}.computed.txt", fixture.name), computed).expect("write");
        std::fs::write(format!("{out_dir}/{}.declared.txt", fixture.name), declared).expect("write");
        eprintln!("dumped {}", fixture.name);
    }
}
