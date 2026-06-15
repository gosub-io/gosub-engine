#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
/// Renders a simple HTML page through the gosub pipeline and saves the result as a PNG.
///
/// Each element's computed box is painted with its CSS background-color. Text nodes are
/// rendered as a semi-transparent dark bar (placeholder — real text rasterisation requires
/// a font backend not available here).
///
/// Run with:
///   cargo run --example screenshot -p gosub_render_pipeline
use std::sync::Arc;

use gosub_css3::system::Css3System;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::html_compile;
use gosub_html5::parser::Html5Parser;
use gosub_interface::config::ModuleConfiguration;
use gosub_interface::css3::CssSystem as _;
use gosub_interface::document::Document as _;
use image::{ImageBuffer, Rgba, RgbaImage};

use gosub_render_pipeline::common::document::pipeline_doc::GosubDocumentAdapter;
use gosub_render_pipeline::common::document::style::{StyleProperty, Value};
use gosub_render_pipeline::common::geo::Dimension;
use gosub_render_pipeline::layouter::taffy::TaffyLayouter;
use gosub_render_pipeline::layouter::{CanLayout, ElementContext};
use gosub_render_pipeline::rendertree_builder::RenderTree;

// ── Config ────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
struct Config;

impl ModuleConfiguration for Config {
    type CssSystem = Css3System;
    type Document = DocumentImpl<Self>;
    type HtmlParser = Html5Parser<'static, Self>;
}

// ── Demo page ─────────────────────────────────────────────────────────────────

const HTML: &str = r#"<!DOCTYPE html>
<html>
<head>
<style>
  body   { background-color: #ecf0f1; margin: 0; padding: 20px; }
  h1     { background-color: #2c3e50; color: white; padding: 12px; margin: 0 0 16px 0; }
  .hero  { background-color: #3498db; height: 60px; margin-bottom: 16px; }
  .row   { margin-bottom: 12px; }
  .card  { background-color: #ffffff; display: inline-block; width: 180px; height: 100px; margin-right: 12px; }
  .red   { background-color: #e74c3c; }
  .green { background-color: #2ecc71; }
  .gold  { background-color: #f39c12; }
</style>
</head>
<body>
  <h1>Gosub Pipeline — layout demo</h1>
  <div class="hero"></div>
  <div class="row">
    <div class="card">Box A</div>
    <div class="card red">Box B</div>
    <div class="card green">Box C</div>
    <div class="card gold">Box D</div>
  </div>
</body>
</html>"#;

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() {
    let width: u32 = 800;
    let height: u32 = 600;

    // 1. Parse HTML + attach UA stylesheet so display values are computed.
    let mut doc = html_compile::<Config>(HTML);
    let ua = Css3System::load_default_useragent_stylesheet();
    doc.add_stylesheet(ua);

    // 2. Build filtered render tree (invisible elements removed).
    let adapter = GosubDocumentAdapter::<Config>::new(Arc::new(doc));
    let mut render_tree = RenderTree::new(Arc::new(adapter));
    render_tree.parse().expect("failed to build render tree");
    let element_count = render_tree.count_elements();

    // 3. Run taffy layout.
    let mut layouter = TaffyLayouter::new();
    let layout_tree = layouter.layout(render_tree, Some(Dimension::new(width as f64, height as f64)), 1.0);

    // 4. Walk layout tree and paint each element into a pixel buffer.
    let mut img: RgbaImage = ImageBuffer::from_pixel(width, height, Rgba([236, 240, 241, 255]));

    let root_id = layout_tree.root_id;
    let mut stack = vec![root_id];
    while let Some(id) = stack.pop() {
        let Some(el) = layout_tree.get_node_by_id(id) else {
            continue;
        };

        match &el.context {
            ElementContext::None => {
                let bg = layout_tree
                    .render_tree
                    .doc
                    .get_style(el.dom_node_id, &StyleProperty::BackgroundColor);
                if let Value::Color(r, g, b, a) = bg {
                    let rgba = [r, g, b, a];
                    if rgba[3] > 0 {
                        let bb = &el.box_model.border_box;
                        fill_rect(
                            &mut img,
                            bb.x as f32,
                            bb.y as f32,
                            bb.width as f32,
                            bb.height as f32,
                            rgba,
                        );
                    }
                }
            }
            ElementContext::Text(ctx) => {
                if !ctx.text.trim().is_empty() {
                    // Approximate text as a dark semi-transparent bar sized to the font.
                    let bb = &el.box_model.content_box;
                    let bar_h = (ctx.font_info.size as f32 * 0.6).max(4.0);
                    fill_rect(
                        &mut img,
                        bb.x as f32,
                        (bb.y + ctx.text_offset.y) as f32,
                        (bb.width as f32).max(8.0),
                        bar_h,
                        [40, 40, 40, 200],
                    );
                }
            }
            ElementContext::Image(_) | ElementContext::Svg(_) => {}
        }

        for &child_id in el.children.iter().rev() {
            stack.push(child_id);
        }
    }

    let out = "pipeline-screenshot.png";
    img.save(out).expect("failed to save PNG");

    println!("gosub pipeline screenshot");
    println!("  HTML elements in render tree : {element_count}");
    println!("  Layout nodes computed        : {}", layout_tree.arena.len());
    println!("  Viewport                     : {width}x{height}");
    println!("  Output                       : {out}");
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn fill_rect(img: &mut RgbaImage, x: f32, y: f32, w: f32, h: f32, color: [u8; 4]) {
    let x0 = (x.max(0.0) as u32).min(img.width());
    let y0 = (y.max(0.0) as u32).min(img.height());
    let x1 = ((x + w).max(0.0) as u32).min(img.width());
    let y1 = ((y + h).max(0.0) as u32).min(img.height());
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    for py in y0..y1 {
        for px in x0..x1 {
            let [r, g, b, a] = color;
            if a == 255 {
                img.put_pixel(px, py, Rgba([r, g, b, 255]));
            } else {
                // Simple alpha-blend over existing pixel.
                let Rgba([br, bg, bb, _]) = *img.get_pixel(px, py);
                let fa = a as f32 / 255.0;
                let fr = r as f32 / 255.0;
                let fg = g as f32 / 255.0;
                let fb = b as f32 / 255.0;
                img.put_pixel(
                    px,
                    py,
                    Rgba([
                        ((fr * fa + (br as f32 / 255.0) * (1.0 - fa)) * 255.0) as u8,
                        ((fg * fa + (bg as f32 / 255.0) * (1.0 - fa)) * 255.0) as u8,
                        ((fb * fa + (bb as f32 / 255.0) * (1.0 - fa)) * 255.0) as u8,
                        255,
                    ]),
                );
            }
        }
    }
}
