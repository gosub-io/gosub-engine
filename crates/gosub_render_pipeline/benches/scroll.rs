//! Scroll benchmark: what it costs to keep a laid-out page painted while it scrolls.
//!
//! Run: cargo bench -p gosub_render_pipeline --bench scroll
//!
//! Record a baseline before changing the tiler or the painter, then compare against it after:
//!
//!   cargo bench -p gosub_render_pipeline --bench scroll -- --save-baseline before
//!   cargo bench -p gosub_render_pipeline --bench scroll -- --baseline before
//!
//! Scrolling does not re-style and does not re-lay-out: a scroll moves no box. What it does is
//! move the raster window down the page, which leaves stages 4 and 5 to bring the newly exposed
//! band up to date. That is what this measures, and only that - the document, its styles and its
//! layout are built once per fixture, outside the measured loop.
//!
//! Two measurements per fixture:
//!
//! - `first-window`: the grid is built from scratch and the first raster window painted. What
//!   the user waits for between layout finishing and the page appearing.
//! - `scroll-through`: from that state, scroll to the bottom of the page in 300 px steps. Each
//!   step is one pass of what the engine's extend path does: reset the reused grid, mark the
//!   tiles a previous pass already painted as ready, park everything outside the raster window,
//!   and paint what is left dirty. The shaped-text cache lives on the grid, so it survives the
//!   steps exactly as it does in the engine.
//!
//! Each iteration starts from a fresh grid, and so from a cold shape cache: a reader arriving at
//! a page and scrolling down it, not a grid that has already seen the whole page.
//!
//! Fixtures are the ones the style benchmark and the dump tools use, so all of them describe the
//! same pages:
//!
//! - `small`: a dozen elements and the user-agent sheet. The floor.
//! - `utility-3k`: about 3,000 elements under a generated utility-first sheet.
//! - `wikipedia+2.2m`: the wikipedia fixture under the 2.2 MB real-world sheet.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashSet;
use std::hint::black_box;
use std::sync::Arc;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};
use gosub_css3::stylesheet::{set_layout_viewport, CssStylesheet};
use gosub_css3::system::Css3System;
use gosub_css3::Css3;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::html_compile;
use gosub_html5::parser::Html5Parser;
use gosub_interface::config::ModuleConfiguration;
use gosub_interface::css3::{CssOrigin, CssSystem as _};
use gosub_interface::document::Document as _;
use gosub_interface::font_system::FontSystem;
use gosub_render_pipeline::common::browser_state::{BrowserState, WireframeState};
use gosub_render_pipeline::common::document::pipeline_doc::GosubDocumentAdapter;
use gosub_render_pipeline::common::geo::{Dimension, Rect};
use gosub_render_pipeline::layering::layer::{LayerId, LayerList};
use gosub_render_pipeline::layouter::taffy::TaffyLayouter;
use gosub_render_pipeline::layouter::CanLayout;
use gosub_render_pipeline::painter::Painter;
use gosub_render_pipeline::rendertree_builder::RenderTree;
use gosub_render_pipeline::tile_budget::defer_tiles_outside_window;
use gosub_render_pipeline::tiler::{TileId, TileList, TileState};
use gosub_shared::config::ParserConfig;

// The fixture pages, shared with the style benchmark and the dump tools.
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

const VIEWPORT: (f64, f64) = (1280.0, 800.0);
/// The tile edge the engine defaults to.
const TILE: f64 = 256.0;
/// One wheel notch, near enough. Small enough that a step usually exposes one row of tiles.
const SCROLL_STEP: f64 = 300.0;
/// Cap on steps per iteration, so a very tall fixture does not dominate the run.
const MAX_STEPS: usize = 20;

// ── Fixtures ─────────────────────────────────────────────────────────────────

/// A page laid out once. Everything here is what a scroll does *not* redo.
struct Fixture {
    name: &'static str,
    layer_list: Arc<LayerList>,
    /// Kept alive because the painter shapes text with the same font system the layouter
    /// measured with, which is what makes painted glyphs the measured ones.
    font_system: Arc<parking_lot::Mutex<dyn FontSystem>>,
    page_height: f64,
    /// Scroll offsets visited by one `scroll-through` iteration.
    offsets: Vec<f64>,
}

fn parse_author_sheet(css: &str, name: &str) -> CssStylesheet {
    let config = ParserConfig {
        ignore_errors: true,
        ..Default::default()
    };
    Css3::parse_str(css, config, CssOrigin::Author, name).expect("bench stylesheet should parse")
}

fn build_fixture(name: &'static str, html: &str, author_css: &[(&str, &str)]) -> Fixture {
    set_layout_viewport(VIEWPORT.0 as f32, VIEWPORT.1 as f32);

    let mut doc = html_compile::<Config>(html);
    doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());
    for (sheet_name, css) in author_css {
        doc.add_stylesheet(parse_author_sheet(css, sheet_name));
    }
    let node_count = doc.node_count();

    // Stages 1-2, once. A scroll re-runs neither.
    let adapter = GosubDocumentAdapter::<Config>::new(Arc::new(doc));
    let mut render_tree = RenderTree::new(Arc::new(adapter));
    render_tree.parse().expect("render tree should build");

    let mut layouter = TaffyLayouter::new();
    let layout_tree = layouter.layout(render_tree, Some(Dimension::new(VIEWPORT.0, VIEWPORT.1)), 1.0);
    let page_height = layout_tree.root_dimension.height.max(VIEWPORT.1);

    let layer_list = Arc::new(LayerList::new(Arc::new(layout_tree)));

    // Stop at the bottom of the page rather than scrolling past it: a step beyond the end
    // exposes nothing and would measure an empty pass.
    let last = (page_height - VIEWPORT.1).max(0.0);
    let mut offsets = Vec::new();
    let mut y = SCROLL_STEP;
    while y <= last && offsets.len() < MAX_STEPS {
        offsets.push(y);
        y += SCROLL_STEP;
    }

    eprintln!(
        "fixture {name}: {node_count} nodes, page {page_height:.0}px, {} scroll step(s)",
        offsets.len()
    );

    Fixture {
        name,
        layer_list,
        font_system: layouter.font_system(),
        page_height,
        offsets,
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

// ── The pass under test ──────────────────────────────────────────────────────

/// The whole tile grid, in page coordinates. The engine paints across the grid's full width
/// rather than the viewport's, so a page wider than the viewport keeps every column.
fn full_page_rect(fixture: &Fixture) -> Rect {
    let page_width = fixture.layer_list.layout_tree.root_dimension.width.max(VIEWPORT.0);
    Rect::new(0.0, 0.0, page_width, fixture.page_height)
}

/// Paint every dirty tile, and report which ones were painted. Mirrors the engine's
/// `paint_dirty_tiles`, including the `note_painted` report a reused grid depends on.
fn paint_dirty(fixture: &Fixture, tile_list: &mut TileList, layer_ids: &[LayerId], rect: Rect) -> Vec<TileId> {
    let paint_state = BrowserState {
        visible_layer_list: vec![true; layer_ids.len()],
        wireframed: WireframeState::None,
        debug_hover: false,
        current_hovered_element: None,
        show_tilegrid: false,
        debug_table_cells: false,
        viewport: rect,
        tile_list: None,
        dpi_scale_factor: 1.0,
    };
    // A fresh painter per pass, as the engine builds one per pass: its per-element memo is
    // per-pass, and only the shaped text outlives the pass, through the grid's cache.
    let painter = Painter::new(tile_list.layer_list.clone(), Some(Arc::clone(&fixture.font_system)))
        .with_shape_cache(Arc::clone(&tile_list.shape_cache));

    let mut painted = Vec::new();
    for &layer_id in layer_ids {
        for tile_id in tile_list.get_intersecting_tiles(layer_id, rect) {
            let Some(tile) = tile_list.get_tile_mut(tile_id) else {
                continue;
            };
            if tile.state != TileState::Dirty {
                continue;
            }
            for tiled_element in &mut tile.elements {
                tiled_element.paint_commands = painter.paint(tiled_element, &paint_state);
            }
            painted.push(tile_id);
        }
    }
    tile_list.note_painted(painted.clone());
    painted
}

/// Stage 4 from scratch plus the first painted window: what the grid costs to build, and what
/// the first screenful costs to paint.
fn first_window(fixture: &Fixture) -> usize {
    let mut tile_list = TileList::from_arc(Arc::clone(&fixture.layer_list), Dimension::new(TILE, TILE));
    tile_list.generate();

    let layer_ids = tile_list.layer_list.layer_ids.read().clone();
    defer_tiles_outside_window(&mut tile_list, 0.0, VIEWPORT.1);
    paint_dirty(fixture, &mut tile_list, &layer_ids, full_page_rect(fixture)).len()
}

/// The state a scroll starts from: a generated grid with the first window already painted.
fn at_top(fixture: &Fixture) -> (TileList, HashSet<TileId>) {
    let mut tile_list = TileList::from_arc(Arc::clone(&fixture.layer_list), Dimension::new(TILE, TILE));
    tile_list.generate();

    let layer_ids = tile_list.layer_list.layer_ids.read().clone();
    defer_tiles_outside_window(&mut tile_list, 0.0, VIEWPORT.1);
    let painted = paint_dirty(fixture, &mut tile_list, &layer_ids, full_page_rect(fixture));
    (tile_list, painted.into_iter().collect())
}

/// One scroll step, as the engine's extend path runs it: keep the grid, hold on to what is
/// already painted, park what the window no longer covers, paint the rest.
fn extend_pass(fixture: &Fixture, tile_list: &mut TileList, done: &mut HashSet<TileId>, scroll_y: f64) -> usize {
    // Stage 4: the grid is a pure function of the layer list and the tile size, neither of
    // which a scroll touches, so it is reset rather than rebuilt.
    tile_list.reset_states();
    // What earlier passes painted is still good; the engine recognises these by page position
    // among the tiles it has baked, which for a reused grid is the same set.
    for &tile_id in done.iter() {
        if let Some(tile) = tile_list.get_tile_mut(tile_id) {
            tile.state = TileState::Ready;
        }
    }
    defer_tiles_outside_window(tile_list, scroll_y, VIEWPORT.1);

    let layer_ids = tile_list.layer_list.layer_ids.read().clone();
    let painted = paint_dirty(fixture, tile_list, &layer_ids, full_page_rect(fixture));
    let count = painted.len();
    done.extend(painted);
    count
}

fn scroll_through(fixture: &Fixture, tile_list: &mut TileList, done: &mut HashSet<TileId>) -> usize {
    let mut painted = 0;
    for &scroll_y in &fixture.offsets {
        painted += extend_pass(fixture, tile_list, done, scroll_y);
    }
    painted
}

// ── The measurements ─────────────────────────────────────────────────────────

fn bench_scroll(c: &mut Criterion) {
    let fixtures = load_fixtures();

    let mut group = c.benchmark_group("scroll/first-window");
    group.sample_size(10).measurement_time(Duration::from_secs(15));
    for fixture in &fixtures {
        group.bench_with_input(BenchmarkId::from_parameter(fixture.name), fixture, |b, fixture| {
            b.iter(|| black_box(first_window(fixture)));
        });
    }
    group.finish();

    let mut group = c.benchmark_group("scroll/scroll-through");
    group.sample_size(10).measurement_time(Duration::from_secs(20));
    for fixture in &fixtures {
        if fixture.offsets.is_empty() {
            // A page that fits in the viewport never scrolls; measuring it would time an
            // empty loop and report it as if it were a scroll.
            continue;
        }
        group.throughput(Throughput::Elements(fixture.offsets.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(fixture.name), fixture, |b, fixture| {
            // The grid is rebuilt per iteration, outside the measurement: every iteration
            // starts at the top of the page with a cold shape cache.
            b.iter_batched_ref(
                || at_top(fixture),
                |(tile_list, done)| black_box(scroll_through(fixture, tile_list, done)),
                BatchSize::LargeInput,
            );
        });
    }
    group.finish();
}

criterion_group!(benches, bench_scroll);
criterion_main!(benches);
