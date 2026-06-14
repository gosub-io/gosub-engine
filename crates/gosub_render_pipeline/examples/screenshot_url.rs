#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
/// Fetch a URL, run it through the gosub pipeline, and save the result as a PNG.
///
/// Elements are painted with their CSS background-color. Text nodes are rendered
/// using Pango so actual glyphs appear in the output. Inline `<img>`/`<svg>` and CSS
/// `background-image` are decoded via the pipeline's media store and blitted (raster scaled,
/// SVG rasterized). Background placement is approximated as centered-contain — exact
/// `background-size: cover`/`repeat` is not reproduced by this example.
///
/// Usage: cargo run --example screenshot_url -p gosub_render_pipeline --features backend_cairo -- <url> [width] [height]
///   cargo run --example screenshot_url -p gosub_render_pipeline --features backend_cairo -- https://example.com
///   cargo run --example screenshot_url -p gosub_render_pipeline --features backend_cairo -- https://example.com 1280 900
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

use gosub_render_pipeline::common::document::pipeline_doc::GosubDocumentAdapter;
use gosub_render_pipeline::common::document::style::{StyleProperty, Value};
use gosub_render_pipeline::common::geo::Dimension;
use gosub_render_pipeline::common::media::DecodedImage;
use gosub_render_pipeline::layouter::taffy::TaffyLayouter;
use gosub_render_pipeline::layouter::{BackgroundMedia, CanLayout, ElementContext};
use gosub_render_pipeline::rendertree_builder::RenderTree;

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
        eprintln!("Usage: screenshot_url <url> [width] [output.png]");
        std::process::exit(1);
    }

    let url_str = &args[1];
    let url = Url::parse(url_str).unwrap_or_else(|e| {
        eprintln!("Invalid URL '{url_str}': {e}");
        std::process::exit(1);
    });

    let width: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1280);
    let out_path = args.get(3).map(|s| s.as_str()).unwrap_or("pipeline-screenshot.png");

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
    render_tree.parse().expect("failed to build render tree");
    let element_count = render_tree.count_elements();
    eprintln!("  {element_count} nodes in render tree");

    // 4. Run taffy layout.
    // Height is unconstrained (MaxContent) so the layout expands to the full page height.
    // We pass a large sentinel value; the actual rendered height is read back from
    // layout_tree.root_dimension after layout completes.
    eprintln!("Running layout (taffy)…");
    let mut layouter = TaffyLayouter::new();
    let layout_tree = layouter.layout(render_tree, Some(Dimension::new(width as f64, 1_000_000.0)), 1.0);
    eprintln!("  {} layout nodes computed", layout_tree.arena.len());
    // Media (images/SVGs) referenced by the page were decoded into this store during layout.
    let media_store = layouter.media_store();

    // Derive the actual page height from the root element's computed size.
    let height = (layout_tree.root_dimension.height.ceil() as u32).max(1);
    eprintln!("  Page dimensions              : {width}x{height}");

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
                let doc = &layout_tree.render_tree.doc;
                let bb = &el.box_model.border_box;

                let bg =
                    if let Value::Color(r, g, b, a) = doc.get_style(el.dom_node_id, &StyleProperty::BackgroundColor) {
                        Some([r, g, b, a])
                    } else {
                        None
                    };

                let bw_top = doc.get_style_f32(el.dom_node_id, &StyleProperty::BorderTopWidth) as f64;
                let bw_right = doc.get_style_f32(el.dom_node_id, &StyleProperty::BorderRightWidth) as f64;
                let bw_bottom = doc.get_style_f32(el.dom_node_id, &StyleProperty::BorderBottomWidth) as f64;
                let bw_left = doc.get_style_f32(el.dom_node_id, &StyleProperty::BorderLeftWidth) as f64;

                let color_of = |prop: &StyleProperty| -> [u8; 4] {
                    if let Value::Color(r, g, b, a) = doc.get_style(el.dom_node_id, prop) {
                        [r, g, b, a]
                    } else {
                        [0u8, 0u8, 0u8, 0u8]
                    }
                };
                let bc_top = color_of(&StyleProperty::BorderTopColor);
                let bc_right = color_of(&StyleProperty::BorderRightColor);
                let bc_bottom = color_of(&StyleProperty::BorderBottomColor);
                let bc_left = color_of(&StyleProperty::BorderLeftColor);

                let r_tl = doc.get_style_f32(el.dom_node_id, &StyleProperty::BorderTopLeftRadius) as f64;
                let r_tr = doc.get_style_f32(el.dom_node_id, &StyleProperty::BorderTopRightRadius) as f64;
                let r_br = doc.get_style_f32(el.dom_node_id, &StyleProperty::BorderBottomRightRadius) as f64;
                let r_bl = doc.get_style_f32(el.dom_node_id, &StyleProperty::BorderBottomLeftRadius) as f64;

                render_box_cairo(
                    &mut img,
                    bb.x as f32,
                    bb.y as f32,
                    bb.width as f32,
                    bb.height as f32,
                    bg,
                    [bw_top, bw_right, bw_bottom, bw_left],
                    [bc_top, bc_right, bc_bottom, bc_left],
                    r_tl,
                    r_tr,
                    r_br,
                    r_bl,
                );
            }
            ElementContext::Text(ctx) => {
                if !ctx.text.trim().is_empty() {
                    let bb = &el.box_model.content_box;
                    let color = layout_tree
                        .render_tree
                        .doc
                        .get_style(el.dom_node_id, &StyleProperty::Color);
                    let [tr, tg, tb, ta] = match color {
                        Value::Color(r, g, b, a) => [r, g, b, a],
                        _ => [0u8, 0u8, 0u8, 255u8],
                    };

                    render_text_pango(
                        &mut img,
                        ctx.text.as_str(),
                        bb.x as f32 + ctx.text_offset.x as f32,
                        bb.y as f32 + ctx.text_offset.y as f32,
                        bb.width as f32,
                        bb.height as f32,
                        ctx.font_info.family.as_str(),
                        ctx.font_info.size as f32,
                        ctx.font_info.weight,
                        [tr, tg, tb, ta],
                    );
                }
            }
            ElementContext::Image(ctx) => {
                // Replaced <img>: the content box already holds the final display size, so scale
                // the decoded raster to fit it.
                let bb = &el.box_model.content_box;
                let media = media_store.get_image(ctx.media_id);
                blit_image_contain(
                    &mut img,
                    bb.x as f32,
                    bb.y as f32,
                    bb.width as f32,
                    bb.height as f32,
                    &media.image,
                );
            }
            ElementContext::Svg(ctx) => {
                let bb = &el.box_model.content_box;
                let media = media_store.get_svg(ctx.media_id);
                blit_svg_contain(
                    &mut img,
                    bb.x as f32,
                    bb.y as f32,
                    bb.width as f32,
                    bb.height as f32,
                    &media.svg.tree,
                );
            }
        }

        // CSS background-image (resolved into the media store during layout). Painted after the
        // element's own background-color/border and before its children, so it sits behind content.
        if let Some(bg) = el.background_media {
            let bb = &el.box_model.border_box;
            match bg {
                BackgroundMedia::Image(media_id) => {
                    let media = media_store.get_image(media_id);
                    blit_image_contain(
                        &mut img,
                        bb.x as f32,
                        bb.y as f32,
                        bb.width as f32,
                        bb.height as f32,
                        &media.image,
                    );
                }
                BackgroundMedia::Svg(media_id) => {
                    let media = media_store.get_svg(media_id);
                    blit_svg_contain(
                        &mut img,
                        bb.x as f32,
                        bb.y as f32,
                        bb.width as f32,
                        bb.height as f32,
                        &media.svg.tree,
                    );
                }
            }
        }

        for &child_id in el.children.iter().rev() {
            stack.push(child_id);
        }
    }

    // 6. Save PNG.
    let out = out_path;
    img.save_with_format(out, image::ImageFormat::Png)
        .expect("failed to save PNG");

    eprintln!("Done.");
    println!("gosub pipeline screenshot");
    println!("  URL                          : {url}");
    println!("  HTML elements in render tree : {element_count}");
    println!("  Layout nodes computed        : {}", layout_tree.arena.len());
    println!("  Dimensions                   : {width}x{height}");
    println!("  Output                       : {out}");
}

fn fetch_html(url: &str) -> anyhow::Result<String> {
    let parsed = url::Url::parse(url)?;
    let response = gosub_net::net::simple::sync_fetch(&parsed)?;
    Ok(String::from_utf8_lossy(&response.body).into_owned())
}

// ── Image rendering (raster + SVG) ───────────────────────────────────────────

/// Src-over composite a straight-alpha RGBA `src` onto the opaque `img` at `(dst_x, dst_y)`.
fn composite(img: &mut RgbaImage, dst_x: i64, dst_y: i64, src: &RgbaImage) {
    for sy in 0..src.height() {
        let py = dst_y + sy as i64;
        if py < 0 || py >= img.height() as i64 {
            continue;
        }
        for sx in 0..src.width() {
            let px = dst_x + sx as i64;
            if px < 0 || px >= img.width() as i64 {
                continue;
            }
            let Rgba([sr, sg, sb, sa]) = *src.get_pixel(sx, sy);
            if sa == 0 {
                continue;
            }
            let fa = sa as f32 / 255.0;
            let Rgba([dr, dg, db, _]) = *img.get_pixel(px as u32, py as u32);
            img.put_pixel(
                px as u32,
                py as u32,
                Rgba([
                    (sr as f32 * fa + dr as f32 * (1.0 - fa)) as u8,
                    (sg as f32 * fa + dg as f32 * (1.0 - fa)) as u8,
                    (sb as f32 * fa + db as f32 * (1.0 - fa)) as u8,
                    255,
                ]),
            );
        }
    }
}

/// Scale `src` to fit within `w`×`h` preserving aspect ratio (`contain`), then composite it
/// centered at `(x, y)`.
fn blit_rgba_contain(img: &mut RgbaImage, x: f32, y: f32, w: f32, h: f32, src: &RgbaImage) {
    if w <= 0.0 || h <= 0.0 || src.width() == 0 || src.height() == 0 {
        return;
    }
    let scale = (w / src.width() as f32).min(h / src.height() as f32);
    let tw = ((src.width() as f32 * scale).round() as u32).max(1);
    let th = ((src.height() as f32 * scale).round() as u32).max(1);
    let resized = image::imageops::resize(src, tw, th, image::imageops::FilterType::Triangle);
    let ox = (x + (w - tw as f32) / 2.0).round() as i64;
    let oy = (y + (h - th as f32) / 2.0).round() as i64;
    composite(img, ox, oy, &resized);
}

/// Blit a decoded raster image into the box at `(x, y, w, h)` as centered-contain.
fn blit_image_contain(img: &mut RgbaImage, x: f32, y: f32, w: f32, h: f32, decoded: &DecodedImage) {
    let Some(src) = ImageBuffer::from_raw(decoded.width(), decoded.height(), decoded.as_raw().to_vec()) else {
        return;
    };
    blit_rgba_contain(img, x, y, w, h, &src);
}

/// Rasterize an SVG tree to fit within the box (centered-contain) and blit it.
fn blit_svg_contain(img: &mut RgbaImage, x: f32, y: f32, w: f32, h: f32, tree: &resvg::usvg::Tree) {
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let size = tree.size();
    let (sw, sh) = (size.width(), size.height());
    if sw <= 0.0 || sh <= 0.0 {
        return;
    }
    // Rasterize at the contained pixel size so the SVG stays crisp (no post-scale blur).
    let scale = (w / sw).min(h / sh);
    let tw = ((sw * scale).round() as u32).max(1);
    let th = ((sh * scale).round() as u32).max(1);
    let Some(mut pixmap) = resvg::tiny_skia::Pixmap::new(tw, th) else {
        return;
    };
    resvg::render(
        tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );

    // tiny_skia stores premultiplied alpha; un-premultiply into straight-alpha RGBA.
    let mut src = RgbaImage::new(tw, th);
    for (i, px) in pixmap.pixels().iter().enumerate() {
        let a = px.alpha();
        let (r, g, b) = if a == 0 {
            (0, 0, 0)
        } else {
            let inv = 255.0 / a as f32;
            (
                (px.red() as f32 * inv).round().min(255.0) as u8,
                (px.green() as f32 * inv).round().min(255.0) as u8,
                (px.blue() as f32 * inv).round().min(255.0) as u8,
            )
        };
        src.put_pixel(i as u32 % tw, i as u32 / tw, Rgba([r, g, b, a]));
    }

    let ox = (x + (w - tw as f32) / 2.0).round() as i64;
    let oy = (y + (h - th as f32) / 2.0).round() as i64;
    composite(img, ox, oy, &src);
}

// ── Box rendering (background + border + rounded corners) ────────────────────

/// `border_widths` = [top, right, bottom, left]  `border_colors` = [top, right, bottom, left]
#[allow(clippy::too_many_arguments)]
fn render_box_cairo(
    img: &mut RgbaImage,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    bg: Option<[u8; 4]>,
    border_widths: [f64; 4],
    border_colors: [[u8; 4]; 4],
    r_tl: f64,
    r_tr: f64,
    r_br: f64,
    r_bl: f64,
) {
    use cairo::{Context, Format, ImageSurface};
    use std::f64::consts::PI;

    let sw = w.ceil() as i32;
    let sh = h.ceil() as i32;
    if sw <= 0 || sh <= 0 {
        return;
    }

    let has_bg = bg.is_some_and(|c| c[3] > 0);
    let is_rounded = r_tl > 0.0 || r_tr > 0.0 || r_br > 0.0 || r_bl > 0.0;
    // Any side with a non-zero width and non-transparent colour counts as a border.
    let has_any_border = border_widths
        .iter()
        .zip(border_colors.iter())
        .any(|(&bw, bc)| bw > 0.0 && bc[3] > 0);

    if !has_bg && !has_any_border {
        return;
    }

    let Ok(mut surface) = ImageSurface::create(Format::ARgb32, sw, sh) else {
        return;
    };
    let Ok(cr) = Context::new(&surface) else { return };

    let fw = w as f64;
    let fh = h as f64;

    // Helper: build the rounded-rectangle path used for fills and uniform borders.
    let build_rounded_path = |cr: &Context| {
        cr.new_path();
        cr.move_to(r_tl, 0.0);
        cr.line_to(fw - r_tr, 0.0);
        if r_tr > 0.0 {
            cr.arc(fw - r_tr, r_tr, r_tr, -PI / 2.0, 0.0);
        } else {
            cr.line_to(fw, 0.0);
        }
        cr.line_to(fw, fh - r_br);
        if r_br > 0.0 {
            cr.arc(fw - r_br, fh - r_br, r_br, 0.0, PI / 2.0);
        } else {
            cr.line_to(fw, fh);
        }
        cr.line_to(r_bl, fh);
        if r_bl > 0.0 {
            cr.arc(r_bl, fh - r_bl, r_bl, PI / 2.0, PI);
        } else {
            cr.line_to(0.0, fh);
        }
        cr.line_to(0.0, r_tl);
        if r_tl > 0.0 {
            cr.arc(r_tl, r_tl, r_tl, PI, 3.0 * PI / 2.0);
        } else {
            cr.line_to(0.0, 0.0);
        }
        cr.close_path();
    };

    // Fill background using the rounded path.
    if has_bg {
        build_rounded_path(&cr);
        let [r, g, b, a] = bg.unwrap();
        cr.set_source_rgba(r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0, a as f64 / 255.0);
        let _ = cr.fill();
    }

    // Draw borders. For rounded boxes (or uniform borders) use the rounded path as a stroke.
    // For asymmetric borders (e.g. border-bottom-only), draw each side as an individual line
    // so we don't accidentally paint borders on sides that have width=0.
    if has_any_border {
        let [bw_t, bw_r, bw_b, bw_l] = border_widths;
        let all_same_width = (bw_t - bw_r).abs() < 0.1 && (bw_t - bw_b).abs() < 0.1 && (bw_t - bw_l).abs() < 0.1;
        let all_same_color = border_colors[0] == border_colors[1]
            && border_colors[0] == border_colors[2]
            && border_colors[0] == border_colors[3];

        if is_rounded || (all_same_width && all_same_color && bw_t > 0.0 && border_colors[0][3] > 0) {
            // Rounded corners, or a uniform 4-side border — use the rounded path.
            build_rounded_path(&cr);
            let [r, g, b, a] = border_colors[0];
            cr.set_source_rgba(r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0, a as f64 / 255.0);
            cr.set_line_width(bw_t.max(bw_r).max(bw_b).max(bw_l));
            let _ = cr.stroke();
        } else {
            // Asymmetric borders: draw only the sides with non-zero width.
            let draw_side = |cr: &Context, x0: f64, y0: f64, x1: f64, y1: f64, bw: f64, bc: [u8; 4]| {
                if bw <= 0.0 || bc[3] == 0 {
                    return;
                }
                let [r, g, b, a] = bc;
                cr.set_source_rgba(r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0, a as f64 / 255.0);
                cr.set_line_width(bw);
                cr.move_to(x0, y0);
                cr.line_to(x1, y1);
                let _ = cr.stroke();
            };
            let hw_t = bw_t / 2.0;
            let hw_r = bw_r / 2.0;
            let hw_b = bw_b / 2.0;
            let hw_l = bw_l / 2.0;
            draw_side(&cr, 0.0, hw_t, fw, hw_t, bw_t, border_colors[0]); // top
            draw_side(&cr, fw - hw_r, 0.0, fw - hw_r, fh, bw_r, border_colors[1]); // right
            draw_side(&cr, 0.0, fh - hw_b, fw, fh - hw_b, bw_b, border_colors[2]); // bottom
            draw_side(&cr, hw_l, 0.0, hw_l, fh, bw_l, border_colors[3]); // left
        }
    }
    drop(cr);
    surface.flush();

    // Composite into the image (Cairo ARGB32 on LE: bytes are [B, G, R, A])
    let Ok(data) = surface.data() else { return };
    let ix0 = (x as i32).max(0) as u32;
    let iy0 = (y as i32).max(0) as u32;
    let ix1 = ((x + w) as u32).min(img.width());
    let iy1 = ((y + h) as u32).min(img.height());

    for py in iy0..iy1 {
        for px in ix0..ix1 {
            let sx = (px - ix0) as usize;
            let sy = (py - iy0) as usize;
            let base = (sy * sw as usize + sx) * 4;
            if base + 3 >= data.len() {
                continue;
            }
            let sa = data[base + 3];
            if sa == 0 {
                continue;
            }
            let sb = data[base];
            let sg = data[base + 1];
            let sr = data[base + 2];
            let fa = sa as f32 / 255.0;
            let Rgba([dr, dg, db, _]) = *img.get_pixel(px, py);
            img.put_pixel(
                px,
                py,
                Rgba([
                    (sr as f32 * fa + dr as f32 * (1.0 - fa)) as u8,
                    (sg as f32 * fa + dg as f32 * (1.0 - fa)) as u8,
                    (sb as f32 * fa + db as f32 * (1.0 - fa)) as u8,
                    255,
                ]),
            );
        }
    }
}

// ── Text rendering via Pango ──────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn render_text_pango(
    img: &mut RgbaImage,
    text: &str,
    x: f32,
    y: f32,
    max_width: f32,
    max_height: f32,
    family: &str,
    size_px: f32,
    weight: i32,
    color: [u8; 4],
) {
    use cairo::{Context, Format, ImageSurface};
    use pangocairo::functions::{context_set_resolution, create_layout, show_layout};
    use pangocairo::pango::{FontDescription, WrapMode, SCALE};

    // Parley (layout) and Pango (raster) use different font metrics, so Pango
    // often measures slightly wider than the box that taffy allocated.  Give the
    // Cairo surface extra horizontal room so glyphs are never clipped; we
    // composite only the pixels that fall inside the original bounding box anyway.
    let layout_w = max_width.ceil() as i32; // the wrap / clip width for Pango
    let surf_w = layout_w + 64; // extra pixels so the last glyph isn't cut
    let surf_h = (max_height.ceil() as i32 + 4).max(1);
    if layout_w <= 0 {
        return;
    }

    let Ok(mut surface) = ImageSurface::create(Format::ARgb32, surf_w, surf_h) else {
        return;
    };
    let Ok(cr) = Context::new(&surface) else {
        return;
    };

    // Transparent background
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);
    let _ = cr.paint();

    // Text colour
    cr.set_source_rgba(
        color[0] as f64 / 255.0,
        color[1] as f64 / 255.0,
        color[2] as f64 / 255.0,
        color[3] as f64 / 255.0,
    );

    let layout = create_layout(&cr);
    // 72 DPI makes 1 Pango point = 1 CSS pixel, so set_size(px × SCALE) is exact.
    // This matches the Cairo pipeline renderer which also uses 72 DPI.
    context_set_resolution(&layout.context(), 72.0);

    let mut font_desc = FontDescription::new();
    font_desc.set_family(family);
    font_desc.set_size((size_px * SCALE as f32) as i32);
    // Map CSS weight 100–900 to Pango weight
    let pango_weight = match weight {
        0..=149 => pangocairo::pango::Weight::Thin,
        150..=249 => pangocairo::pango::Weight::Ultralight,
        250..=349 => pangocairo::pango::Weight::Light,
        350..=449 => pangocairo::pango::Weight::Normal,
        450..=549 => pangocairo::pango::Weight::Medium,
        550..=649 => pangocairo::pango::Weight::Semibold,
        650..=749 => pangocairo::pango::Weight::Bold,
        750..=849 => pangocairo::pango::Weight::Ultrabold,
        _ => pangocairo::pango::Weight::Heavy,
    };
    font_desc.set_weight(pango_weight);
    layout.set_font_description(Some(&font_desc));
    layout.set_text(text);
    // Use the original content-box width for wrapping, not the padded surface width.
    layout.set_width(layout_w * SCALE);
    layout.set_wrap(WrapMode::Word);

    cr.move_to(0.0, 0.0);
    show_layout(&cr, &layout);
    drop(cr);

    // Flush and read back pixels
    surface.flush();
    let Ok(data) = surface.data() else { return };

    // Composite Cairo's ARGB32 (premultiplied, BGRA on LE) into the RGBA image
    let ix0 = (x as i32).max(0) as u32;
    let iy0 = (y as i32).max(0) as u32;
    // Clip composite region to the original content-box width, not the padded surface.
    let ix1 = ((x + layout_w as f32) as u32).min(img.width());
    let iy1 = ((y + surf_h as f32) as u32).min(img.height());

    for py in iy0..iy1 {
        for px in ix0..ix1 {
            let sx = (px - ix0) as usize;
            let sy = (py - iy0) as usize;
            let base = (sy * surf_w as usize + sx) * 4;
            if base + 3 >= data.len() {
                continue;
            }
            // Cairo ARGB32 on little-endian: [B, G, R, A] bytes = pixel as u32 0xAARRGGBB
            let a = data[base + 3];
            if a == 0 {
                continue;
            }
            let b = data[base];
            let g = data[base + 1];
            let r = data[base + 2];
            let fa = a as f32 / 255.0;
            let Rgba([br, bg, bb, _]) = *img.get_pixel(px, py);
            img.put_pixel(
                px,
                py,
                Rgba([
                    (r as f32 * fa + br as f32 * (1.0 - fa)) as u8,
                    (g as f32 * fa + bg as f32 * (1.0 - fa)) as u8,
                    (b as f32 * fa + bb as f32 * (1.0 - fa)) as u8,
                    255,
                ]),
            );
        }
    }
}
