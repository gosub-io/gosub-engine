//! The renderer role: the render pipeline, run inside a forked, confined
//! child.
//!
//! [`RetainedPage`] is a parsed, styled, laid-out page kept between renders:
//! a one-shot renderer builds one and renders the whole page, a resident
//! renderer keeps it and renders only the raster window around the viewport,
//! rasterizing more as the viewport moves, repainting the few tiles a hover
//! touches, and letting go of tiles that drift too far.

use crate::fork_server::protocol::{HitRegion, PageSummary, MAX_HIT_REGIONS};
use crate::html::{EngineDocument, RenderConfiguration};
use gosub_html5::document::builder::DocumentBuilderImpl;
use gosub_html5::parser::{Html5Parser, Html5ParserOptions};
use gosub_interface::css3::CssSystem as _;
use gosub_interface::document::Document as _;
use gosub_interface::font_system::FontSystem;
use gosub_interface::resource_loader::ResourceLoader;
use gosub_render_pipeline::common::document::pipeline_doc::GosubDocumentAdapter;
use gosub_render_pipeline::common::geo::{Dimension, Rect};
use gosub_render_pipeline::layering::layer::{LayerId, LayerList};
use gosub_render_pipeline::rasterizer::{BakedTile, Rasterable};
use gosub_render_pipeline::render::backend::TileAnchor;
use gosub_render_pipeline::tile_budget::{defer_tiles_outside_window, raster_window, TilePosKey};
use gosub_render_pipeline::tiler::{TileList, TileState};
use gosub_shared::byte_stream::{ByteStream, Encoding};
use gosub_shared::node::NodeId;
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use url::Url;

/// Tile edge in CSS pixels, matching the engine's default.
const TILE_SIZE: f64 = 256.0;

/// How far (in viewport heights) from the raster window a shipped tile may
/// drift before a retained page lets go of it. Wider than the window's own
/// margin so ordinary back-and-forth scrolling reuses tiles rather than
/// re-shipping them.
pub const EVICT_MARGIN_VIEWPORTS: f64 = 3.0;

/// Parse, style, lay out, layer, tile, paint - and, when the configuration
/// provides a forked rasterizer, rasterize - the whole of `html`, measuring
/// and shaping through `fonts`. Pure compute plus allocation: safe under the
/// strictest renderer filter. Returns the summary and the baked tiles (empty
/// without a rasterizer); sealing them into memfds is the caller's business,
/// since that is transport, not rendering.
pub fn render_page<C: RenderConfiguration>(
    page: PageRequest<'_>,
    fonts: Arc<Mutex<dyn FontSystem>>,
    media_store: Arc<gosub_render_pipeline::common::media::MediaStore>,
    loader: Arc<dyn ResourceLoader>,
) -> (PageSummary, Vec<RenderedTile>, Vec<HitRegion>) {
    let known_tiles = page.known_tiles;
    let mut retained = RetainedPage::build::<C>(page, fonts, media_store, loader);
    let pass = retained.render(None, known_tiles);
    (pass.summary, pass.tiles, retained.hit_regions)
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
    /// The DOM node under the pointer, for `:hover` styling.
    pub hovered_node: Option<u64>,
}

/// One tile as the renderer decided to handle it, in composite order.
pub enum RenderedTile {
    /// Rasterized here; its pixels must be shipped.
    Fresh { tile: BakedTile, hash: u64 },
    /// The broker already holds this tile's pixels: nothing was rasterized
    /// and nothing travels but the identity. The broker fills the physical
    /// dimensions from what it kept - the renderer never produced them.
    Unchanged {
        page_x: f64,
        page_y: f64,
        layer_id: u64,
        hash: u64,
    },
}

/// What one render pass produced.
pub struct RenderPass {
    pub summary: PageSummary,
    pub tiles: Vec<RenderedTile>,
    /// Content hashes of tiles the broker held that this page no longer
    /// accounts for.
    pub evicted: Vec<u64>,
}

/// A tile the broker holds from this page.
struct Shipped {
    hash: u64,
    rect: Rect,
    /// Viewport-pinned tiles are never far from the viewport.
    scrolls: bool,
}

/// A page after stages 1-3: everything up to the layer list, retained so
/// later passes can tile, paint and rasterize any window of it without
/// parsing again.
pub struct RetainedPage {
    title: Option<String>,
    favicon: Option<String>,
    layer_list: Arc<LayerList>,
    layer_ids: Vec<LayerId>,
    fonts: Arc<Mutex<dyn FontSystem>>,
    media_store: Arc<gosub_render_pipeline::common::media::MediaStore>,
    rasterizer: Option<Box<dyn Rasterable + Send + Sync>>,
    viewport_width: f64,
    viewport_height: f64,
    page_width: f64,
    page_height: f64,
    /// The page's hit-test geometry, fixed at layout time.
    pub hit_regions: Vec<HitRegion>,
    /// What the broker holds of this page, by tile position: the tiles a
    /// pass need not produce, and the pool eviction draws from.
    shipped: HashMap<TilePosKey, Shipped>,
    /// Where the viewport was last: a hover repaint stays within that window.
    scroll_y: f64,
    /// The DOM node under the pointer, as the broker last told us.
    hovered: Option<NodeId>,
    /// Moves the document's `:hover` state; the document itself sits behind
    /// the pipeline's document adapter, which does not expose that.
    set_hovered: Box<dyn Fn(Option<NodeId>) + Send + Sync>,
    /// Stage costs of `build`, reported with the first pass only.
    build_timings: Vec<(String, u64)>,
}

impl RetainedPage {
    /// Stages 1-3 over `page`: parse (subresources through `loader`), apply
    /// hover, register web fonts, build the render tree, lay out, layer.
    pub fn build<C: RenderConfiguration>(
        page: PageRequest<'_>,
        fonts: Arc<Mutex<dyn FontSystem>>,
        media_store: Arc<gosub_render_pipeline::common::media::MediaStore>,
        loader: Arc<dyn ResourceLoader>,
    ) -> Self {
        use gosub_render_pipeline::layouter::taffy::TaffyLayouter;
        use gosub_render_pipeline::layouter::CanLayout;
        use gosub_render_pipeline::rendertree_builder::RenderTree;

        let PageRequest {
            html,
            page_url,
            viewport_width,
            viewport_height,
            hovered_node,
            ..
        } = page;

        // Viewport-relative CSS units resolve against the real viewport; must
        // precede parse(), which computes styles for display:none filtering.
        gosub_css3::stylesheet::set_layout_viewport(viewport_width as f32, viewport_height as f32);

        // Parse with the page's base URL (relative subresource URLs resolve
        // against it) and with the loader, so `<link rel="stylesheet">` is
        // fetched through the broker mid-parse - the same arrangement as the
        // engine's own parse, with the renderer's brokered loader in the seat.
        let started = std::time::Instant::now();
        let base_url = Url::parse(page_url).ok();
        let mut stream = ByteStream::from_str(html, Encoding::UTF8);
        let mut doc = DocumentBuilderImpl::new_document::<C>(base_url.clone());
        let parser_options = Html5ParserOptions {
            resource_loader: Some(Arc::clone(&loader)),
            ..Default::default()
        };
        let _ = Html5Parser::<C>::parse_document(&mut stream, &mut doc, Some(parser_options));
        doc.add_stylesheet(C::CssSystem::load_default_useragent_stylesheet());

        // Hover state, as the broker hit-tested it. Applied before the render
        // tree is built, which is when styles (including `:hover`) are computed.
        let hovered = hovered_node.map(NodeId::from);
        if hovered.is_some() {
            doc.set_hovered_nodes(hovered);
        }

        // `@font-face` web fonts: the same walk the tab worker runs, fetching
        // through this renderer's loader and registering into the inherited font
        // system - so text set in a web font lays out here exactly as in-process.
        if let Some(base) = &base_url {
            crate::html::web_fonts::load_web_fonts::<C>(&doc, base, loader.as_ref(), &mut |bytes, family| {
                fonts.lock().register_font(bytes, Some(family))
            });
        }
        let parse_us = started.elapsed().as_micros() as u64;
        let title = crate::html::document_title::<C>(&doc);
        let favicon = base_url
            .as_ref()
            .and_then(|base| crate::html::favicon_url::<C>(&doc, base))
            .map(|url| url.to_string());

        // Stage 1: render tree.
        let doc = Arc::new(doc);
        let doc_for_regions = Arc::clone(&doc);
        let set_hovered: Box<dyn Fn(Option<NodeId>) + Send + Sync> = {
            let doc = Arc::clone(&doc);
            Box::new(move |leaf| doc.set_hovered_nodes(leaf))
        };
        let adapter = GosubDocumentAdapter::<C>::new(doc);
        let mut render_tree = RenderTree::new(Arc::new(adapter));
        if let Err(e) = render_tree.parse() {
            // Same degradation as the engine: the layouter tolerates a rootless
            // tree and the page renders empty.
            log::error!("failed to build render tree in the forked renderer: {e}");
        }
        let render_tree_us = started.elapsed().as_micros() as u64 - parse_us;

        // Stage 2: layout, measured through the inherited font system. The
        // media store must be passed at construction: `with_font_system` builds
        // a private default store first, which is exactly the filesystem-touching
        // construction this process can no longer do.
        let mut layouter =
            TaffyLayouter::with_font_system_and_media_store(Arc::clone(&fonts), Arc::clone(&media_store));
        let vp_dim =
            (viewport_width > 0.0 && viewport_height > 0.0).then(|| Dimension::new(viewport_width, viewport_height));
        let layout_tree = layouter.layout(render_tree, vp_dim, 1.0);
        let page_width = layout_tree.root_dimension.width;
        let page_height = layout_tree.root_dimension.height;
        let layout_us = started.elapsed().as_micros() as u64 - parse_us - render_tree_us;

        // Stage 3: layering.
        let layer_list = Arc::new(LayerList::new(layout_tree));
        let layer_ids = layer_list.layer_ids.read().clone();
        let hit_regions = collect_hit_regions::<C>(&layer_list, &doc_for_regions, base_url.as_ref());
        let build_timings = vec![
            ("build.parse".to_string(), parse_us),
            ("build.render_tree".to_string(), render_tree_us),
            ("build.layout".to_string(), layout_us),
        ];

        Self {
            title,
            favicon,
            layer_list,
            layer_ids,
            rasterizer: C::forked_tile_rasterizer(Arc::clone(&fonts)),
            fonts,
            media_store,
            viewport_width,
            viewport_height,
            page_width,
            page_height,
            hit_regions,
            shipped: HashMap::new(),
            scroll_y: 0.0,
            hovered,
            set_hovered,
            build_timings,
        }
    }

    /// Stages 4-6 over one window of the page: the raster window around
    /// `scroll_y`, or the whole page for `None`. Tiles the broker already
    /// holds - from an earlier pass of this page, or (by content hash) from
    /// `known_tiles` - are neither rasterized nor shipped. With a window,
    /// shipped tiles that now lie further than [`EVICT_MARGIN_VIEWPORTS`]
    /// from it are given up and reported as evicted.
    pub fn render(&mut self, scroll_y: Option<f64>, known_tiles: &HashSet<u64>) -> RenderPass {
        if let Some(scroll_y) = scroll_y {
            self.scroll_y = scroll_y;
        }
        self.pass(scroll_y, known_tiles, None)
    }

    /// The pointer moved to `node` (a DOM node id, or nothing): restyle the
    /// old and new hover chains and repaint only the tiles those elements
    /// cover, within the current raster window. No re-layout - the same
    /// simplification as the in-process hover repaint. Tiles whose painted
    /// content comes out identical are not shipped again.
    pub fn hover(&mut self, node: Option<u64>) -> RenderPass {
        let new_leaf = node.map(NodeId::from);
        let old_leaf = self.hovered;
        if new_leaf == old_leaf {
            return self.empty_pass();
        }
        self.hovered = new_leaf;

        let doc = &self.layer_list.layout_tree.render_tree.doc;
        // Only the two ancestor chains can gain or lose `:hover`.
        let mut dirty_nodes: Vec<NodeId> = Vec::new();
        let mut seen = HashSet::new();
        for start in [old_leaf, new_leaf].into_iter().flatten() {
            let mut id = start;
            loop {
                if seen.insert(id) {
                    dirty_nodes.push(id);
                }
                match doc.parent(id) {
                    Some(parent) => id = parent,
                    None => break,
                }
            }
        }
        (self.set_hovered)(new_leaf);
        doc.invalidate_style_for_nodes(&dirty_nodes);

        // Everything either element covers, as laid out.
        let mut repaint: Option<Rect> = None;
        for element in self.layer_list.layout_tree.arena.values() {
            let matches = [old_leaf, new_leaf]
                .into_iter()
                .flatten()
                .any(|leaf| element.dom_node_id == leaf);
            if !matches {
                continue;
            }
            let m = &element.box_model.margin_box;
            let r = Rect::new(m.x, m.y, m.width, m.height);
            repaint = Some(match repaint {
                None => r,
                Some(u) => {
                    let x0 = u.x.min(r.x);
                    let y0 = u.y.min(r.y);
                    let x1 = (u.x + u.width).max(r.x + r.width);
                    let y1 = (u.y + u.height).max(r.y + r.height);
                    Rect::new(x0, y0, x1 - x0, y1 - y0)
                }
            });
        }
        let Some(repaint) = repaint else {
            return self.empty_pass();
        };
        self.pass(Some(self.scroll_y), &HashSet::new(), Some(repaint))
    }

    fn empty_pass(&self) -> RenderPass {
        RenderPass {
            summary: self.summary(0, 0, Vec::new()),
            tiles: Vec::new(),
            evicted: Vec::new(),
        }
    }

    fn summary(&self, painted_tiles: u64, paint_commands: u64, timings_us: Vec<(String, u64)>) -> PageSummary {
        PageSummary {
            title: self.title.clone(),
            favicon: self.favicon.clone(),
            page_width: self.page_width,
            page_height: self.page_height,
            layer_count: self.layer_ids.len() as u64,
            painted_tiles,
            paint_commands,
            layer_order: self.layer_ids.iter().map(|id| id.as_u64()).collect(),
            timings_us,
        }
    }

    /// One pass: tile, decide which tiles need paint, paint, hash, rasterize,
    /// evict. `repaint` narrows the work to tiles overlapping it (a hover),
    /// forcing those even if the broker holds them; otherwise a tile the
    /// broker already holds by position needs nothing.
    fn pass(&mut self, scroll_y: Option<f64>, known_tiles: &HashSet<u64>, repaint: Option<Rect>) -> RenderPass {
        use gosub_render_pipeline::common::browser_state::{BrowserState, WireframeState};
        use gosub_render_pipeline::painter::Painter;

        let started = std::time::Instant::now();
        let mut timings = std::mem::take(&mut self.build_timings);
        let lap = |name: &str, timings: &mut Vec<(String, u64)>, since: &mut u64| {
            let now = started.elapsed().as_micros() as u64;
            timings.push((name.to_string(), now - *since));
            *since = now;
        };
        let mut since = 0u64;

        // Stage 4: tiling, from the retained layer list.
        let mut tile_list = TileList::from_arc(Arc::clone(&self.layer_list), Dimension::new(TILE_SIZE, TILE_SIZE));
        tile_list.generate();
        if let Some(scroll_y) = scroll_y {
            defer_tiles_outside_window(&mut tile_list, scroll_y, self.viewport_height);
        }

        for tile in tile_list.arena.values_mut() {
            if tile.state != TileState::Dirty {
                continue;
            }
            let key = (tile.rect.x.to_bits(), tile.rect.y.to_bits(), tile.layer_id.as_u64());
            let needs_paint = match repaint {
                Some(area) => {
                    tile.rect.x < area.x + area.width
                        && tile.rect.x + tile.rect.width > area.x
                        && tile.rect.y < area.y + area.height
                        && tile.rect.y + tile.rect.height > area.y
                }
                None => !self.shipped.contains_key(&key),
            };
            if !needs_paint {
                tile.state = TileState::Ready;
            }
        }
        lap("render.tiling", &mut timings, &mut since);

        // Stage 5: paint what is still dirty.
        let full_page_rect = Rect::new(0.0, 0.0, self.viewport_width, self.page_height.max(1.0));
        let paint_state = BrowserState {
            visible_layer_list: vec![true; self.layer_ids.len()],
            wireframed: WireframeState::None,
            debug_hover: false,
            current_hovered_element: None,
            show_tilegrid: false,
            debug_table_cells: false,
            viewport: full_page_rect,
            tile_list: None,
            dpi_scale_factor: 1.0,
        };
        let painter = Painter::new(Arc::clone(&tile_list.layer_list), Some(Arc::clone(&self.fonts)));
        let mut painted_tiles: u64 = 0;
        let mut paint_commands: u64 = 0;
        for &layer_id in &self.layer_ids {
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
        lap("render.paint", &mut timings, &mut since);

        // Between painting and rasterizing: a tile's hash covers its position,
        // layer and painted content, so a hit in `known_tiles` - or the same
        // hash the broker already holds for this position - means the pixels
        // would come out byte-identical: no reason to rasterize it, let alone
        // ship it. Marking such a tile non-dirty is what makes stage 6 skip it.
        let mut plan: Vec<TilePlan> = Vec::new();
        for &layer_id in &self.layer_ids {
            let scrolls = self.layer_list.layer_anchor(layer_id) == TileAnchor::Scroll;
            for tile_id in tile_list.get_intersecting_tiles(layer_id, full_page_rect) {
                let Some(tile) = tile_list.get_tile_mut(tile_id) else {
                    continue;
                };
                if tile.state != TileState::Dirty {
                    continue;
                }
                let key = (tile.rect.x.to_bits(), tile.rect.y.to_bits(), layer_id.as_u64());
                let hash = gosub_render_pipeline::rasterizer::tile_content_hash(tile);
                if self.shipped.get(&key).is_some_and(|s| s.hash == hash) {
                    tile.state = TileState::Ready;
                    continue;
                }
                let unchanged = known_tiles.contains(&hash);
                if unchanged {
                    tile.state = TileState::Ready;
                }
                plan.push(TilePlan {
                    key,
                    rect: tile.rect,
                    scrolls,
                    hash,
                    unchanged,
                });
            }
        }

        // Stage 6, when this configuration can rasterize in a forked child.
        // Sequential on purpose: the renderer filter has no `clone`, so the
        // parallel strategy is not merely unwanted here, it is impossible.
        let baked = match &self.rasterizer {
            Some(rasterizer) => {
                let (baked, _tile_cache) = gosub_render_pipeline::rasterizer::rasterize_sequential(
                    rasterizer.as_ref(),
                    &self.layer_ids,
                    &mut tile_list,
                    full_page_rect,
                    &self.media_store,
                );
                baked
            }
            None => Vec::new(),
        };
        lap("render.raster", &mut timings, &mut since);

        // Re-join the freshly baked tiles with the plan, so what leaves this
        // process is in composite order regardless of which tiles were skipped.
        let mut fresh: HashMap<TilePosKey, BakedTile> = baked
            .into_iter()
            .map(|tile| ((tile.page_x.to_bits(), tile.page_y.to_bits(), tile.layer_id), tile))
            .collect();
        let mut tiles: Vec<RenderedTile> = Vec::with_capacity(plan.len());
        let mut evicted = Vec::new();
        for entry in plan {
            let rendered = if entry.unchanged {
                Some(RenderedTile::Unchanged {
                    page_x: entry.rect.x,
                    page_y: entry.rect.y,
                    layer_id: entry.key.2,
                    hash: entry.hash,
                })
            } else {
                fresh
                    .remove(&entry.key)
                    .map(|tile| RenderedTile::Fresh { tile, hash: entry.hash })
            };
            let Some(rendered) = rendered else {
                continue;
            };
            // A repainted position replaces what the broker held there.
            if let Some(previous) = self.shipped.insert(
                entry.key,
                Shipped {
                    hash: entry.hash,
                    rect: entry.rect,
                    scrolls: entry.scrolls,
                },
            ) {
                evicted.push(previous.hash);
            }
            tiles.push(rendered);
        }

        // Let go of what drifted out of reach. Whole-page passes keep
        // everything: there is no viewport to measure distance from.
        if let Some(scroll_y) = scroll_y {
            let (lo, hi) = raster_window(scroll_y, self.viewport_height);
            let margin = EVICT_MARGIN_VIEWPORTS * self.viewport_height;
            let (keep_lo, keep_hi) = (lo - margin, hi + margin);
            self.shipped.retain(|_, shipped| {
                let far =
                    shipped.scrolls && (shipped.rect.y + shipped.rect.height <= keep_lo || shipped.rect.y >= keep_hi);
                if far {
                    evicted.push(shipped.hash);
                }
                !far
            });
        }

        RenderPass {
            summary: self.summary(painted_tiles, paint_commands, timings),
            tiles,
            evicted,
        }
    }
}

/// Bookkeeping between the paint and rasterize stages: what each tile is and
/// whether the broker already has it.
struct TilePlan {
    key: TilePosKey,
    rect: Rect,
    scrolls: bool,
    hash: u64,
    unchanged: bool,
}

/// Flatten the layer list into hit-test geometry for the broker.
/// Everything the broker needs to answer hit tests and hovers without a DOM
/// of its own, resolved here per box.
fn describe_hit<C: RenderConfiguration>(
    doc: &EngineDocument<C>,
    node: NodeId,
    base_url: Option<&Url>,
) -> (
    Option<String>,
    Option<String>,
    crate::fork_server::protocol::HitCursor,
    bool,
) {
    use crate::fork_server::protocol::HitCursor;
    use gosub_interface::node::NodeType;
    let resolve = |raw: &str| base_url.and_then(|b| b.join(raw).ok()).map(|u| u.to_string());
    let mut link = None;
    let mut image = None;
    let mut editable = false;
    let mut cursor = if doc.node_type(node) == NodeType::TextNode {
        HitCursor::Text
    } else {
        HitCursor::Default
    };
    let mut id = Some(node);
    while let Some(current) = id {
        if link.is_none() && doc.tag_name(current) == Some("a") {
            if let Some(href) = doc.attribute(current, "href") {
                link = Some(resolve(href).unwrap_or_else(|| href.to_string()));
                cursor = HitCursor::Pointer;
            }
        }
        if image.is_none() && doc.tag_name(current) == Some("img") {
            image = doc
                .attribute(current, "src")
                .map(|src| resolve(src).unwrap_or_else(|| src.to_string()));
        }
        if crate::html::is_text_input::<C>(doc, current) {
            editable = true;
            if cursor != HitCursor::Pointer {
                cursor = HitCursor::Text;
            }
        }
        id = doc.parent(current);
    }
    (link, image, cursor, editable)
}

fn collect_hit_regions<C: RenderConfiguration>(
    layer_list: &LayerList,
    doc: &EngineDocument<C>,
    base_url: Option<&Url>,
) -> Vec<HitRegion> {
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
            let (link, image, cursor, editable) = describe_hit::<C>(doc, element.dom_node_id, base_url);
            regions.push(HitRegion {
                x: margin.x,
                y: margin.y,
                width: margin.width,
                height: margin.height,
                node_id: element.dom_node_id.into(),
                anchor: layer.anchor.into(),
                link,
                image,
                cursor,
                editable,
            });
        }
    }
    regions
}
