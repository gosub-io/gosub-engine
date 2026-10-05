//! Style resolution benchmark: what it costs to give every element its computed style.
//!
//! Run: cargo bench -p gosub_render_pipeline --bench style
//!
//! Record a baseline before changing the style system, then compare against it after:
//!
//!   cargo bench -p gosub_render_pipeline --bench style -- --save-baseline before
//!   cargo bench -p gosub_render_pipeline --bench style -- --baseline before
//!
//! Two measurements per fixture, so a change can be attributed to the CSS crate or to the
//! render pipeline's use of it:
//!
//! - `cascade`: the CSS crate alone. Walk the element tree top-down and ask `Css3System` for
//!   each element's property map, given the parent's, then compute every property. This is
//!   exactly what the pipeline adapter does per node, minus the adapter.
//! - `render-tree`: stage 1 of the pipeline, a fresh adapter plus `RenderTree::parse()`. It
//!   styles every node through the adapter's cache and reads `display` back through the typed
//!   conversion, so it also covers the string-to-typed bridge.
//!
//! Parsing the HTML and the stylesheets happens once, outside the measured loop: this measures
//! styling, not parsing, and the media environment is pinned to a 1280x800 viewport.
//!
//! Fixtures:
//!
//! - `small`: a dozen elements and the user-agent sheet. The floor.
//! - `utility-3k`: a generated page of about 3,000 elements, class-heavy, with a generated
//!   utility-first stylesheet of several thousand rules: a `*` reset that sets ~50 custom
//!   properties, spacing/colour/layout utilities, `hover:` and responsive `md:`/`lg:`
//!   variants inside `@media`, descendant component rules, and inline `style` attributes on
//!   some elements. Deterministic, no dependencies. This is the shape of the page that took
//!   12 s to style before the custom-property sharing fix.
//! - `wikipedia+2.2m`: the wikipedia fixture DOM with the 2.2 MB real-world sheet from
//!   `tests/data/css3-data`. About 18,000 rules, almost none of which match: the case that
//!   exercises the selector index and `@media` evaluation rather than the cascade itself.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::hint::black_box;
use std::sync::Arc;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
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
use gosub_render_pipeline::common::document::pipeline_doc::GosubDocumentAdapter;
use gosub_render_pipeline::rendertree_builder::RenderTree;
use gosub_shared::config::ParserConfig;
use gosub_shared::node::NodeId;

// The fixture pages, shared with the `style_dump` example so a dump compares exactly the page
// this measures.
#[path = "common/pages.rs"]
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

// ── Fixtures ─────────────────────────────────────────────────────────────────

struct Fixture {
    name: &'static str,
    doc: Arc<DocumentImpl<Config>>,
    /// Elements that receive a computed style in one top-down pass.
    elements: u64,
}

fn parse_author_sheet(css: &str, name: &str) -> CssStylesheet {
    let config = ParserConfig {
        ignore_errors: true,
        ..Default::default()
    };
    Css3::parse_str(css, config, CssOrigin::Author, name).expect("bench stylesheet should parse")
}

fn build_fixture(name: &'static str, html: &str, author_css: &[(&str, &str)]) -> Fixture {
    let mut doc = html_compile::<Config>(html);
    doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());
    for (sheet_name, css) in author_css {
        doc.add_stylesheet(parse_author_sheet(css, sheet_name));
    }
    let rules: usize = doc.stylesheets().iter().map(|s| s.rules.len()).sum();

    set_layout_viewport(VIEWPORT.0, VIEWPORT.1);
    let elements = cascade_document(&doc);
    eprintln!(
        "fixture {name}: {} nodes, {elements} styled elements, {rules} rules",
        doc.node_count()
    );

    Fixture {
        name,
        doc: Arc::new(doc),
        elements,
    }
}

fn load_fixtures() -> Vec<Fixture> {
    let data_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/data");
    let wikipedia = std::fs::read_to_string(format!("{data_dir}/tree_iterator/wikipedia_main.html"))
        .expect("tests/data/tree_iterator/wikipedia_main.html should exist");
    let real_world_css = std::fs::read_to_string(format!("{data_dir}/css3-data/data.css"))
        .expect("tests/data/css3-data/data.css should exist");

    let utility = generate_utility_page();

    vec![
        build_fixture("small", SMALL_HTML, &[]),
        build_fixture("utility-3k", &utility.html, &[("utility.css", &utility.css)]),
        build_fixture("wikipedia+2.2m", &wikipedia, &[("data.css", &real_world_css)]),
    ]
}

// ── The two measurements ──────────────────────────────────────────────────────

/// Style every element top-down through the CSS crate alone. Returns how many got a map.
fn cascade_document(doc: &DocumentImpl<Config>) -> u64 {
    let mut styled = 0;
    let root = doc.root();
    style_subtree(doc, root, doc.stylesheets(), None, &mut styled);
    styled
}

fn style_subtree(
    doc: &DocumentImpl<Config>,
    id: NodeId,
    sheets: &[CssStylesheet],
    parent: Option<&CssProperties>,
    styled: &mut u64,
) {
    let own = if doc.node_type(id) == NodeType::ElementNode {
        let mut map = Css3System::properties_from_node::<Config>(doc, id, sheets, parent);
        if let Some(map) = map.as_mut() {
            // The adapter computes every property right after the cascade; so does this.
            for (_, property) in map.iter_mut() {
                black_box(property.compute_value());
            }
            *styled += 1;
        }
        map
    } else {
        None
    };

    // Text nodes take no style of their own; their children (none) and the element's children
    // inherit from the nearest element, which is what the adapter's flat-parent lookup does.
    let inherited = own.as_ref().or(parent);
    for &child in doc.children(id) {
        style_subtree(doc, child, sheets, inherited, styled);
    }
}

/// Stage 1 of the pipeline: a cold adapter and a render tree built over it.
fn build_render_tree(doc: &Arc<DocumentImpl<Config>>) -> usize {
    let adapter = GosubDocumentAdapter::<Config>::new(Arc::clone(doc));
    let mut render_tree = RenderTree::new(Arc::new(adapter));
    render_tree.parse().expect("render tree should build");
    render_tree.count_elements()
}

fn bench_style(c: &mut Criterion) {
    let fixtures = load_fixtures();

    let mut group = c.benchmark_group("style/cascade");
    group.sample_size(10).measurement_time(Duration::from_secs(15));
    for fixture in &fixtures {
        group.throughput(Throughput::Elements(fixture.elements));
        group.bench_with_input(BenchmarkId::from_parameter(fixture.name), fixture, |b, fixture| {
            set_layout_viewport(VIEWPORT.0, VIEWPORT.1);
            b.iter(|| black_box(cascade_document(&fixture.doc)));
        });
    }
    group.finish();

    let mut group = c.benchmark_group("style/render-tree");
    group.sample_size(10).measurement_time(Duration::from_secs(15));
    for fixture in &fixtures {
        group.throughput(Throughput::Elements(fixture.elements));
        group.bench_with_input(BenchmarkId::from_parameter(fixture.name), fixture, |b, fixture| {
            set_layout_viewport(VIEWPORT.0, VIEWPORT.1);
            b.iter(|| black_box(build_render_tree(&fixture.doc)));
        });
    }
    group.finish();
}

criterion_group!(benches, bench_style);
criterion_main!(benches);
