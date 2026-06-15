#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
/// Pipeline benchmark — runs stages 1-5 against several HTML fixtures and prints
/// per-stage timing with mean ± stddev across N iterations.
///
/// Usage:
///   cargo run --example pipeline_bench -p gosub_render_pipeline
///   cargo run --example pipeline_bench -p gosub_render_pipeline -- --iterations 20
///
/// Stages timed:
///   1. render-tree   — DOM → filtered RenderTree
///   2. layout        — RenderTree → LayoutTree (Taffy)
///   3. layering      — LayoutTree → LayerList
///   4. tiling        — LayerList → TileList (256×256 grid)
///   5. painting      — TileList → PaintCommands (all dirty tiles)
use std::sync::Arc;
use std::time::{Duration, Instant};

use gosub_css3::system::Css3System;
use gosub_html5::document::builder::DocumentBuilderImpl;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::parser::Html5Parser;
use gosub_interface::config::ModuleConfiguration;
use gosub_interface::css3::CssSystem as _;
use gosub_interface::document::Document as _;
use gosub_shared::byte_stream::{ByteStream, Encoding};

use gosub_render_pipeline::common::browser_state::{BrowserState, WireframeState};
use gosub_render_pipeline::common::document::pipeline_doc::GosubDocumentAdapter;
use gosub_render_pipeline::common::geo::{Dimension, Rect};
use gosub_render_pipeline::layering::layer::LayerList;
use gosub_render_pipeline::layouter::taffy::TaffyLayouter;
use gosub_render_pipeline::layouter::CanLayout;
use gosub_render_pipeline::painter::Painter;
use gosub_render_pipeline::rendertree_builder::RenderTree;
use gosub_render_pipeline::tiler::{TileList, TileState};

// ── Config ─────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
struct Config;

impl ModuleConfiguration for Config {
    type CssSystem = Css3System;
    type Document = DocumentImpl<Self>;
    type HtmlParser = Html5Parser<'static, Self>;
}

// ── Stats ──────────────────────────────────────────────────────────────────

fn mean_stddev(samples: &[Duration]) -> (f64, f64) {
    let n = samples.len() as f64;
    let mean = samples.iter().map(|d| d.as_secs_f64() * 1000.0).sum::<f64>() / n;
    let variance = samples
        .iter()
        .map(|d| {
            let diff = d.as_secs_f64() * 1000.0 - mean;
            diff * diff
        })
        .sum::<f64>()
        / n;
    (mean, variance.sqrt())
}

// ── HTML source ────────────────────────────────────────────────────────────

struct Fixture {
    name: &'static str,
    html: String,
}

fn load_fixtures() -> Vec<Fixture> {
    // Paths are relative to the workspace root (where `cargo run` is invoked from).
    let candidates: &[(&str, &str)] = &[
        ("wikipedia", "tests/data/tree_iterator/wikipedia_main.html"),
        ("stackoverflow", "tests/data/tree_iterator/stackoverflow.html"),
    ];

    let mut fixtures: Vec<Fixture> = candidates
        .iter()
        .filter_map(|(name, path)| std::fs::read_to_string(path).ok().map(|html| Fixture { name, html }))
        .collect();

    // Always include a small synthetic document so there is something to run even
    // when the fixture files are not found.
    fixtures.push(Fixture {
        name: "synthetic-small",
        html: SMALL_HTML.to_string(),
    });

    fixtures
}

const SMALL_HTML: &str = r#"<!DOCTYPE html>
<html>
<head>
<style>
  body  { background: #eee; margin: 0; padding: 16px; }
  h1    { background: #333; color: #fff; padding: 8px; }
  .card { background: #fff; display: inline-block; width: 150px; height: 80px; margin: 8px; }
  .red  { background: #e74c3c; }
  .blue { background: #3498db; }
</style>
</head>
<body>
  <h1>Benchmark page</h1>
  <p>Lorem ipsum dolor sit amet.</p>
  <div class="card">A</div><div class="card red">B</div><div class="card blue">C</div>
  <div class="card">D</div><div class="card red">E</div><div class="card blue">F</div>
</body>
</html>"#;

// ── Pipeline runner ────────────────────────────────────────────────────────

struct StageTimes {
    render_tree: Duration,
    layout: Duration,
    layering: Duration,
    tiling: Duration,
    painting: Duration,
}

fn run_pipeline_once(html: &str, layouter: &mut TaffyLayouter) -> StageTimes {
    let viewport_w = 1280.0_f64;
    let viewport_h = 800.0_f64;

    // Parse HTML+CSS outside of the measured stages (it's a pre-requisite, not
    // part of the rendering pipeline itself).
    let mut stream = ByteStream::from_str(html, Encoding::UTF8);
    let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
    let _ = Html5Parser::<Config>::parse_document(&mut stream, &mut doc, None);
    let ua = Css3System::load_default_useragent_stylesheet();
    doc.add_stylesheet(ua);
    let doc_arc = Arc::new(doc);

    // Stage 1: render tree
    let t = Instant::now();
    let adapter = GosubDocumentAdapter::<Config>::new(Arc::clone(&doc_arc));
    let mut render_tree = RenderTree::new(Arc::new(adapter));
    render_tree.parse().expect("failed to build render tree");
    let render_tree_time = t.elapsed();

    // Stage 2: layout
    let t = Instant::now();
    let layout_tree = layouter.layout(render_tree, Some(Dimension::new(viewport_w, viewport_h)), 1.0);
    let layout_time = t.elapsed();

    let page_height = layout_tree.root_dimension.height;

    // Stage 3: layering
    let t = Instant::now();
    let layer_list = LayerList::new(layout_tree);
    let layering_time = t.elapsed();

    // Stage 4: tiling
    let t = Instant::now();
    let mut tile_list = TileList::new(layer_list, Dimension::new(256.0, 256.0));
    tile_list.generate();
    let tiling_time = t.elapsed();

    // Stage 5: painting (all dirty tiles, full-page viewport)
    let t = Instant::now();
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
    let painter = Painter::new(tile_list.layer_list.clone());
    for &layer_id in &layer_ids {
        let tile_ids = tile_list.get_intersecting_tiles(layer_id, full_rect);
        for tile_id in tile_ids {
            if let Some(tile) = tile_list.get_tile_mut(tile_id) {
                if tile.state == TileState::Dirty {
                    for tiled_element in &mut tile.elements {
                        tiled_element.paint_commands = painter.paint(tiled_element, &paint_state);
                    }
                }
            }
        }
    }
    let painting_time = t.elapsed();

    StageTimes {
        render_tree: render_tree_time,
        layout: layout_time,
        layering: layering_time,
        tiling: tiling_time,
        painting: painting_time,
    }
}

// ── Main ───────────────────────────────────────────────────────────────────

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let iterations: usize = args
        .iter()
        .position(|a| a == "--iterations" || a == "-n")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);

    let fixtures = load_fixtures();

    println!("Gosub render pipeline benchmark");
    println!("  iterations : {iterations}");
    println!(
        "  fixtures   : {}",
        fixtures.iter().map(|f| f.name).collect::<Vec<_>>().join(", ")
    );
    println!();

    for fixture in &fixtures {
        println!(
            "── {} ─────────────────────────────────────────────────────",
            fixture.name
        );

        // Each fixture gets its own layouter so the media store (images, SVGs) is warm
        // for all timed iterations — we measure layout computation, not network I/O.
        let mut layouter = TaffyLayouter::new();

        // Warm up — one throw-away run to prime the media store and OS caches.
        let _ = run_pipeline_once(&fixture.html, &mut layouter);

        let mut rt_times = Vec::with_capacity(iterations);
        let mut layout_times = Vec::with_capacity(iterations);
        let mut layer_times = Vec::with_capacity(iterations);
        let mut tile_times = Vec::with_capacity(iterations);
        let mut paint_times = Vec::with_capacity(iterations);

        for _ in 0..iterations {
            let t = run_pipeline_once(&fixture.html, &mut layouter);
            rt_times.push(t.render_tree);
            layout_times.push(t.layout);
            layer_times.push(t.layering);
            tile_times.push(t.tiling);
            paint_times.push(t.painting);
        }

        let stages: &[(&str, &[Duration])] = &[
            ("1. render-tree", &rt_times),
            ("2. layout      ", &layout_times),
            ("3. layering    ", &layer_times),
            ("4. tiling      ", &tile_times),
            ("5. painting    ", &paint_times),
        ];

        let total_means: f64 = stages.iter().map(|(_, s)| mean_stddev(s).0).sum();

        for (name, samples) in stages {
            let (mean, stddev) = mean_stddev(samples);
            let pct = mean / total_means * 100.0;
            println!("  stage {name}  {:>7.2}ms ± {:>5.2}ms  ({:>4.1}%)", mean, stddev, pct);
        }
        println!("  ─────────────────────────────────────────────────────────────");
        println!("  total (stages 1-5)         {:>7.2}ms", total_means);
        println!();
    }
}
