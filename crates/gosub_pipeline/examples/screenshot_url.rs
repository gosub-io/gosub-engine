/// Fetch a URL, run it through the gosub pipeline, and save the result as a PNG.
///
/// Each element's computed box is painted with its CSS background-color. Text nodes are
/// rendered as a semi-transparent dark bar (placeholder).
///
/// Usage: cargo run --example screenshot_url -p gosub_pipeline -- <url> [width] [height]
///   cargo run --example screenshot_url -p gosub_pipeline -- https://example.com
///   cargo run --example screenshot_url -p gosub_pipeline -- https://example.com 1280 900
use std::sync::Arc;

use gosub_css3::system::Css3System;
use gosub_html5::document::builder::DocumentBuilderImpl;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::parser::Html5Parser;
use gosub_interface::config::{HasCssSystem, HasDocument};
use gosub_interface::css3::CssSystem as _;
use gosub_interface::document::Document as _;
use gosub_shared::byte_stream::{ByteStream, Encoding};
use image::{ImageBuffer, Rgba, RgbaImage};
use url::Url;

use gosub_pipeline::common::document::pipeline_doc::GosubDocumentAdapter;
use gosub_pipeline::common::document::style::{StyleProperty, Value};
use gosub_pipeline::common::geo::Dimension;
use gosub_pipeline::layouter::taffy::TaffyLayouter;
use gosub_pipeline::layouter::{CanLayout, ElementContext};
use gosub_pipeline::rendertree_builder::RenderTree;

// ── Config ────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
struct Config;

impl HasCssSystem for Config {
    type CssSystem = Css3System;
}
impl HasDocument for Config {
    type Document = DocumentImpl<Self>;
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: screenshot_url <url> [width] [height]");
        std::process::exit(1);
    }

    let url_str = &args[1];
    let url = Url::parse(url_str).unwrap_or_else(|e| {
        eprintln!("Invalid URL '{url_str}': {e}");
        std::process::exit(1);
    });

    let width: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1280);
    let height: u32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(900);

    // 1. Fetch HTML.
    eprintln!("Fetching {url}…");
    let html = fetch_html(url_str).unwrap_or_else(|e| {
        eprintln!("Fetch failed: {e}");
        std::process::exit(1);
    });
    eprintln!("  {} bytes received", html.len());

    // 2. Parse HTML + attach UA stylesheet.
    eprintln!("Parsing HTML + CSS…");
    let mut doc = DocumentBuilderImpl::new_document::<Config>(Some(url.clone()));
    let mut stream = ByteStream::from_str(&html, Encoding::UTF8);
    let _ = Html5Parser::<Config>::parse_document(&mut stream, &mut doc, None);
    let ua = Css3System::load_default_useragent_stylesheet();
    doc.add_stylesheet(ua);
    eprintln!("  {} stylesheet(s) attached", doc.stylesheets().len());

    // 3. Build render tree.
    eprintln!("Building render tree…");
    let adapter = GosubDocumentAdapter::<Config>::new(Arc::new(doc));
    let mut render_tree = RenderTree::new(Arc::new(adapter));
    render_tree.parse();
    let element_count = render_tree.count_elements();
    eprintln!("  {element_count} nodes in render tree");

    // 4. Run taffy layout.
    eprintln!("Running layout (taffy)…");
    let mut layouter = TaffyLayouter::new();
    let layout_tree = layouter.layout(render_tree, Some(Dimension::new(width as f64, height as f64)), 1.0);
    eprintln!("  {} layout nodes computed", layout_tree.arena.len());

    // 5. Paint into pixel buffer.
    eprintln!("Painting…");
    let mut img: RgbaImage = ImageBuffer::from_pixel(width, height, Rgba([255, 255, 255, 255]));

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

    // 6. Save PNG.
    let out = "pipeline-screenshot.png";
    img.save(out).expect("failed to save PNG");

    eprintln!("Done.");
    println!("gosub pipeline screenshot");
    println!("  URL                          : {url}");
    println!("  HTML elements in render tree : {element_count}");
    println!("  Layout nodes computed        : {}", layout_tree.arena.len());
    println!("  Viewport                     : {width}x{height}");
    println!("  Output                       : {out}");
}

fn fetch_html(url: &str) -> anyhow::Result<String> {
    let parsed = url::Url::parse(url)?;
    let response = gosub_net::net::simple::sync_fetch(&parsed)?;
    Ok(String::from_utf8_lossy(&response.body).into_owned())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn fill_rect(img: &mut RgbaImage, x: f32, y: f32, w: f32, h: f32, color: [u8; 4]) {
    let x0 = (x as u32).min(img.width());
    let y0 = (y as u32).min(img.height());
    let x1 = ((x + w) as u32).min(img.width());
    let y1 = ((y + h) as u32).min(img.height());
    for py in y0..y1 {
        for px in x0..x1 {
            let [r, g, b, a] = color;
            if a == 255 {
                img.put_pixel(px, py, Rgba([r, g, b, 255]));
            } else {
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

