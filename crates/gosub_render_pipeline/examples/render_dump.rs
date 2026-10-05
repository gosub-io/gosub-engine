//! Dump the laid-out boxes and the paint commands of every fixture page, so a change to the
//! style system can be proved to move no pixel.
//!
//! The `style_dump` example proves the CSS crate's *map* did not change. This proves what the
//! render pipeline made of it: stage 1-2 give every element its box, stage 3-5 give every tile
//! its paint commands. Run it before a change and after, and diff the two directories.
//!
//! ```sh
//! cargo run --release -p gosub_render_pipeline --example render_dump -- /tmp/render/before
//! # ... make the change ...
//! cargo run --release -p gosub_render_pipeline --example render_dump -- /tmp/render/after
//! diff -rq /tmp/render/before /tmp/render/after
//! ```
//!
//! Two files per fixture: `<name>.layout.json` is the layouter's own `GOSUB_DUMP_LAYOUT` dump
//! (tag, id, class, depth and border box of every element in document order) and
//! `<name>.paint.txt` the paint commands of every tile of every layer, in layer and tile order.
//!
//! The fixtures are exactly the ones `style_dump` uses, so the three gates describe one thing.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fmt::Write as _;
use std::sync::Arc;

use gosub_css3::stylesheet::{set_layout_viewport, CssStylesheet};
use gosub_css3::system::Css3System;
use gosub_css3::Css3;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::html_compile;
use gosub_html5::parser::Html5Parser;
use gosub_interface::config::ModuleConfiguration;
use gosub_interface::css3::{CssOrigin, CssSystem as _};
use gosub_interface::document::Document as _;
use gosub_shared::config::ParserConfig;

use gosub_render_pipeline::common::browser_state::{BrowserState, WireframeState};
use gosub_render_pipeline::common::document::pipeline_doc::GosubDocumentAdapter;
use gosub_render_pipeline::common::geo::{Dimension, Rect};
use gosub_render_pipeline::layering::layer::LayerList;
use gosub_render_pipeline::layouter::taffy::TaffyLayouter;
use gosub_render_pipeline::layouter::CanLayout;
use gosub_render_pipeline::painter::Painter;
use gosub_render_pipeline::rendertree_builder::RenderTree;
use gosub_render_pipeline::tiler::{TileList, TileState};

// The same fixture pages the style benchmark measures and `style_dump` dumps.
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

/// Stages 1-5 for one fixture. The layout dump is written by the layouter itself, through the
/// `GOSUB_DUMP_LAYOUT` path this sets up; the paint commands come back as text.
fn run_pipeline(doc: DocumentImpl<Config>, layout_path: &str) -> String {
    let viewport_w = 1280.0_f64;
    let viewport_h = 800.0_f64;

    let doc_arc = Arc::new(doc);

    // Stage 1: render tree.
    let adapter = GosubDocumentAdapter::<Config>::new(Arc::clone(&doc_arc));
    let mut render_tree = RenderTree::new(Arc::new(adapter));
    render_tree.parse().expect("render tree should build");

    // Stage 2: layout. The layouter writes the box dump on its way out.
    std::env::set_var("GOSUB_DUMP_LAYOUT", layout_path);
    let mut layouter = TaffyLayouter::new();
    let layout_tree = layouter.layout(render_tree, Some(Dimension::new(viewport_w, viewport_h)), 1.0);
    std::env::remove_var("GOSUB_DUMP_LAYOUT");

    let page_height = layout_tree.root_dimension.height;

    // Stage 3-4: layering and tiling.
    let layer_list = LayerList::new(Arc::new(layout_tree));
    let mut tile_list = TileList::new(layer_list, Dimension::new(256.0, 256.0));
    tile_list.generate();

    // Stage 5: painting, every dirty tile of the whole page.
    let full_rect = Rect::new(0.0, 0.0, viewport_w, page_height.max(viewport_h));
    let layer_ids = tile_list.layer_list.layer_ids.read().clone();
    let paint_state = BrowserState {
        visible_layer_list: vec![true; layer_ids.len()],
        wireframed: WireframeState::None,
        debug_hover: false,
        current_hovered_element: None,
        show_tilegrid: false,
        debug_table_cells: false,
        viewport: full_rect,
        tile_list: None,
        dpi_scale_factor: 1.0,
    };
    let painter = Painter::new(tile_list.layer_list.clone(), Some(layouter.font_system()));

    let mut out = String::new();
    for &layer_id in &layer_ids {
        let tile_ids = tile_list.get_intersecting_tiles(layer_id, full_rect);
        for tile_id in tile_ids {
            let Some(tile) = tile_list.get_tile_mut(tile_id) else {
                continue;
            };
            if tile.state != TileState::Dirty {
                continue;
            }
            let _ = writeln!(out, "layer {layer_id:?} tile {tile_id:?}");
            for tiled_element in &mut tile.elements {
                tiled_element.paint_commands = painter.paint(tiled_element, &paint_state);
                for command in &tiled_element.paint_commands {
                    let _ = writeln!(out, "  {:?} {command:?}", tiled_element.id);
                }
            }
        }
    }
    out
}

fn main() {
    let Some(out_dir) = std::env::args().nth(1) else {
        eprintln!("usage: cargo run -p gosub_render_pipeline --example render_dump -- <out-dir>");
        std::process::exit(2);
    };
    std::fs::create_dir_all(&out_dir).expect("output directory");

    for fixture in fixtures() {
        // Set per fixture as well as once up front: building a fixture parses stylesheets, and
        // the viewport is global state the parse reads.
        set_layout_viewport(VIEWPORT.0, VIEWPORT.1);

        let layout_path = format!("{out_dir}/{}.layout.json", fixture.name);
        let name = fixture.name.clone();
        let paint = run_pipeline(fixture.doc, &layout_path);
        std::fs::write(format!("{out_dir}/{name}.paint.txt"), paint).expect("write");
        eprintln!("dumped {name}");
    }
}
