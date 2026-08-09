//! The renderer role: the render pipeline, run inside a forked, confined
//! child.

use crate::fork_server::protocol::{HitRegion, PageSummary, MAX_HIT_REGIONS};
use crate::html::RenderConfiguration;
use gosub_html5::document::builder::DocumentBuilderImpl;
use gosub_html5::parser::{Html5Parser, Html5ParserOptions};
use gosub_interface::css3::CssSystem as _;
use gosub_interface::document::Document as _;
use gosub_interface::font_system::FontSystem;
use gosub_interface::resource_loader::ResourceLoader;
use gosub_shared::byte_stream::{ByteStream, Encoding};
use parking_lot::Mutex;
use std::collections::HashSet;
use std::sync::Arc;
use url::Url;

/// Tile edge in CSS pixels, matching the engine's default.
const TILE_SIZE: f64 = 256.0;

/// Parse, style, lay out, layer, tile, paint — and, when the configuration
/// provides a forked rasterizer, rasterize — `html`, measuring and shaping
/// through `fonts`. Pure compute plus allocation: safe under the strictest
/// renderer filter. Returns the summary and the baked tiles (empty without a
/// rasterizer); sealing them into memfds is the caller's business, since that
/// is transport, not rendering.
pub fn render_page<C: RenderConfiguration>(
    page: PageRequest<'_>,
    fonts: Arc<Mutex<dyn FontSystem>>,
    media_store: Arc<gosub_render_pipeline::common::media::MediaStore>,
    loader: Arc<dyn ResourceLoader>,
) -> (PageSummary, Vec<RenderedTile>, Vec<HitRegion>) {
    let PageRequest {
        html,
        page_url,
        viewport_width,
        viewport_height,
        known_tiles,
    } = page;
    use gosub_render_pipeline::common::browser_state::{BrowserState, WireframeState};
    use gosub_render_pipeline::common::document::pipeline_doc::GosubDocumentAdapter;
    use gosub_render_pipeline::common::geo::{Dimension, Rect};
    use gosub_render_pipeline::layering::layer::LayerList;
    use gosub_render_pipeline::layouter::taffy::TaffyLayouter;
    use gosub_render_pipeline::layouter::CanLayout;
    use gosub_render_pipeline::painter::Painter;
    use gosub_render_pipeline::rendertree_builder::RenderTree;
    use gosub_render_pipeline::tiler::{TileList, TileState};

    // Viewport-relative CSS units resolve against the real viewport; must
    // precede parse(), which computes styles for display:none filtering.
    gosub_css3::stylesheet::set_layout_viewport(viewport_width as f32, viewport_height as f32);

    // Parse with the page's base URL (relative subresource URLs resolve
    // against it) and with the loader, so `<link rel=\"stylesheet\">` is
    // fetched through the broker mid-parse — the same arrangement as the
    // engine's own parse, with the renderer's brokered loader in the seat.
    let base_url = Url::parse(page_url).ok();
    let mut stream = ByteStream::from_str(html, Encoding::UTF8);
    let mut doc = DocumentBuilderImpl::new_document::<C>(base_url.clone());
    let parser_options = Html5ParserOptions {
        resource_loader: Some(Arc::clone(&loader)),
        ..Default::default()
    };
    let _ = Html5Parser::<C>::parse_document(&mut stream, &mut doc, Some(parser_options));
    doc.add_stylesheet(C::CssSystem::load_default_useragent_stylesheet());

    // `@font-face` web fonts: the same walk the tab worker runs, fetching
    // through this renderer's loader and registering into the inherited font
    // system — so text set in a web font lays out here exactly as in-process.
    if let Some(base) = &base_url {
        crate::html::web_fonts::load_web_fonts::<C>(&doc, base, loader.as_ref(), &mut |bytes, family| {
            fonts.lock().register_font(bytes, Some(family))
        });
    }

    // Stage 1: render tree.
    let adapter = GosubDocumentAdapter::<C>::new(Arc::new(doc));
    let mut render_tree = RenderTree::new(Arc::new(adapter));
    if let Err(e) = render_tree.parse() {
        // Same degradation as the engine: the layouter tolerates a rootless
        // tree and the page renders empty.
        log::error!("failed to build render tree in the forked renderer: {e}");
    }

    // Stage 2: layout, measured through the inherited font system. The
    // media store must be passed at construction: `with_font_system` builds
    // a private default store first, which is exactly the filesystem-touching
    // construction this process can no longer do.
    let mut layouter = TaffyLayouter::with_font_system_and_media_store(Arc::clone(&fonts), Arc::clone(&media_store));
    let vp_dim =
        (viewport_width > 0.0 && viewport_height > 0.0).then(|| Dimension::new(viewport_width, viewport_height));
    let layout_tree = layouter.layout(render_tree, vp_dim, 1.0);
    let page_width = layout_tree.root_dimension.width;
    let page_height = layout_tree.root_dimension.height;

    // Stage 3: layering. Stage 4: tiling.
    let layer_list = LayerList::new(layout_tree);
    let mut tile_list = TileList::new(layer_list, Dimension::new(TILE_SIZE, TILE_SIZE));
    tile_list.generate();

    // Stage 5: paint every tile of the full page, shaping through the same
    // font system layout measured with.
    let full_page_rect = Rect::new(0.0, 0.0, viewport_width, page_height.max(1.0));
    let layer_ids = tile_list.layer_list.layer_ids.read().clone();
    let paint_state = BrowserState {
        visible_layer_list: vec![true; layer_ids.len()],
        wireframed: WireframeState::None,
        debug_hover: false,
        current_hovered_element: None,
        show_tilegrid: false,
        debug_table_cells: false,
        viewport: full_page_rect,
        tile_list: None,
        dpi_scale_factor: 1.0,
    };
    let painter = Painter::new(Arc::clone(&tile_list.layer_list), Some(Arc::clone(&fonts)));

    let mut painted_tiles: u64 = 0;
    let mut paint_commands: u64 = 0;
    for &layer_id in &layer_ids {
        for tile_id in tile_list.get_intersecting_tiles(layer_id, full_page_rect) {
            let Some(tile) = tile_list.get_tile_mut(tile_id) else {
                continue;
            };
            if tile.state != TileState::Dirty {
                continue;
            }
            painted_tiles += 1;
            for tiled_element in &mut tile.elements {
                tiled_element.paint_commands = painter.paint(tiled_element, &paint_state);
                paint_commands += tiled_element.paint_commands.len() as u64;
            }
        }
    }

    // Between painting and rasterizing: decide which tiles the broker already
    // has. A tile's hash covers its position, layer and painted content, so a
    // hit means the pixels would come out byte-identical — no reason to
    // rasterize it, let alone ship it. Marking such a tile non-dirty is what
    // makes stage 6 skip it.
    let mut plan: Vec<TilePlan> = Vec::new();
    for &layer_id in &layer_ids {
        for tile_id in tile_list.get_intersecting_tiles(layer_id, full_page_rect) {
            let Some(tile) = tile_list.get_tile_mut(tile_id) else {
                continue;
            };
            if tile.state != TileState::Dirty {
                continue;
            }
            let hash = gosub_render_pipeline::rasterizer::tile_content_hash(tile);
            let unchanged = known_tiles.contains(&hash);
            if unchanged {
                tile.state = TileState::Ready;
            }
            plan.push(TilePlan {
                page_x: tile.rect.x,
                page_y: tile.rect.y,
                layer_id: layer_id.as_u64(),
                hash,
                unchanged,
            });
        }
    }

    // Stage 6, when this configuration can rasterize in a forked child.
    // Sequential on purpose: the renderer filter has no `clone`, so the
    // parallel strategy is not merely unwanted here, it is impossible.
    let baked = match C::forked_tile_rasterizer(Arc::clone(&fonts)) {
        Some(rasterizer) => {
            let (baked, _tile_cache) = gosub_render_pipeline::rasterizer::rasterize_sequential(
                rasterizer.as_ref(),
                &layer_ids,
                &mut tile_list,
                full_page_rect,
                &media_store,
            );
            baked
        }
        None => Vec::new(),
    };

    // Re-join the freshly baked tiles with the plan, so what leaves this
    // process is in composite order regardless of which tiles were skipped.
    let mut fresh: std::collections::HashMap<(u64, u64, u64), gosub_render_pipeline::rasterizer::BakedTile> = baked
        .into_iter()
        .map(|tile| ((tile.page_x.to_bits(), tile.page_y.to_bits(), tile.layer_id), tile))
        .collect();
    let tiles: Vec<RenderedTile> = plan
        .into_iter()
        .filter_map(|entry| {
            let key = (entry.page_x.to_bits(), entry.page_y.to_bits(), entry.layer_id);
            match entry.unchanged {
                true => Some(RenderedTile::Unchanged {
                    page_x: entry.page_x,
                    page_y: entry.page_y,
                    layer_id: entry.layer_id,
                    hash: entry.hash,
                }),
                false => fresh
                    .remove(&key)
                    .map(|tile| RenderedTile::Fresh { tile, hash: entry.hash }),
            }
        })
        .collect();

    let hit_regions = collect_hit_regions(&tile_list.layer_list);

    (
        PageSummary {
            page_width,
            page_height,
            layer_count: layer_ids.len() as u64,
            painted_tiles,
            paint_commands,
        },
        tiles,
        hit_regions,
    )
}

/// What to render: the page, the viewport it is laid out against, and the
/// tiles the broker already holds.
pub struct PageRequest<'a> {
    pub html: &'a str,
    /// Base URL for the page's relative subresource URLs.
    pub page_url: &'a str,
    pub viewport_width: f64,
    pub viewport_height: f64,
    /// Content hashes the broker kept from a previous render; a tile whose
    /// hash is here is neither rasterized nor shipped.
    pub known_tiles: &'a HashSet<u64>,
}

/// One tile as the renderer decided to handle it, in composite order.
pub enum RenderedTile {
    /// Rasterized here; its pixels must be shipped.
    Fresh {
        tile: gosub_render_pipeline::rasterizer::BakedTile,
        hash: u64,
    },
    /// The broker already holds this tile's pixels: nothing was rasterized
    /// and nothing travels but the identity. The broker fills the physical
    /// dimensions from what it kept — the renderer never produced them.
    Unchanged {
        page_x: f64,
        page_y: f64,
        layer_id: u64,
        hash: u64,
    },
}

/// Bookkeeping between the paint and rasterize stages: what each tile is and
/// whether the broker already has it.
struct TilePlan {
    page_x: f64,
    page_y: f64,
    layer_id: u64,
    hash: u64,
    unchanged: bool,
}

/// Flatten the layer list into hit-test geometry for the broker.
fn collect_hit_regions(layer_list: &gosub_render_pipeline::layering::layer::LayerList) -> Vec<HitRegion> {
    let mut regions = Vec::new();
    let layer_ids = layer_list.layer_ids.read();
    let layers = layer_list.layers.read();

    'outer: for layer_id in layer_ids.iter().rev() {
        let Some(layer) = layers.get(layer_id) else {
            continue;
        };
        for element_id in layer.elements.iter().rev() {
            let Some(element) = layer_list.layout_tree.get_node_by_id(*element_id) else {
                continue;
            };
            if regions.len() >= MAX_HIT_REGIONS {
                log::warn!(
                    "page has more than {MAX_HIT_REGIONS} hit-testable boxes;                      hit testing covers the topmost {MAX_HIT_REGIONS}"
                );
                break 'outer;
            }
            let margin = &element.box_model.margin_box;
            regions.push(HitRegion {
                x: margin.x,
                y: margin.y,
                width: margin.width,
                height: margin.height,
                node_id: element.dom_node_id.into(),
                anchor: layer.anchor.into(),
            });
        }
    }
    regions
}
