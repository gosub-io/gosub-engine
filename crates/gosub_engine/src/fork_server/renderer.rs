//! The renderer role: the render pipeline, run inside a forked, confined
//! child.
//!
//! [`RetainedPage`] is a parsed, styled, laid-out page kept between renders:
//! a one-shot renderer builds one and renders the whole page, a resident
//! renderer keeps it and renders only the raster window around the viewport,
//! rasterizing more as the viewport moves, repainting the few tiles a hover
//! touches, and letting go of tiles that drift too far.

use crate::engine::events::{CursorShape, Modifiers, MouseButton};
use crate::engine::input::{InputHost, KeyOutcome, PageInput};
use crate::fork_server::protocol::{
    Effect, HitCursor, HitRegion, InputEvent, MediaPrefs, PageSummary, WireRect, MAX_HIT_REGIONS,
};
use crate::html::{EngineDocument, RenderConfiguration};
use crate::net::resource_loader::ResourceLoader;
use gosub_html5::document::builder::DocumentBuilderImpl;
use gosub_html5::parser::{Html5Parser, Html5ParserOptions};
use gosub_interface::css3::CssSystem as _;
use gosub_interface::document::Document as _;
use gosub_interface::font_system::FontSystem;
use gosub_interface::node::QuirksMode;
use gosub_render_pipeline::common::document::pipeline_doc::{GosubDocumentAdapter, PipelineDocument};
use gosub_render_pipeline::common::geo::{Dimension, Rect};
use gosub_render_pipeline::layering::layer::{LayerId, LayerList};
use gosub_render_pipeline::layouter::LayoutElementId;
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
    let mut retained = RetainedPage::<C>::build(page, fonts, media_store, loader);
    let pass = retained.render(None, known_tiles);
    (pass.summary, pass.tiles, retained.hit_regions)
}

/// Install what the broker's process-wide render state would have supplied
/// in-process: the device-pixel ratio and the user's media preferences, which
/// `@media` queries, `light-dark()` and the engine-drawn controls read. Once
/// per renderer process, on the thread that renders, before the first page;
/// `set_layout_viewport` keeps the rest of the environment.
pub fn apply_media_prefs(dpr: u32, media: crate::fork_server::protocol::MediaPrefs) {
    use gosub_css3::media_query::{ColorScheme, ReducedMotion};
    gosub_render_pipeline::render::DEVICE_PIXEL_RATIO.store(dpr, std::sync::atomic::Ordering::Relaxed);
    gosub_css3::stylesheet::set_prefers_dark(media.prefers_dark);
    gosub_render_pipeline::common::theme::set_dark(media.prefers_dark);
    let mut env = gosub_css3::media_query::media_environment();
    env.device_pixel_ratio = dpr as f32;
    env.color_scheme = if media.prefers_dark {
        ColorScheme::Dark
    } else {
        ColorScheme::Light
    };
    env.reduced_motion = if media.prefers_reduced_motion {
        ReducedMotion::Reduce
    } else {
        ReducedMotion::NoPreference
    };
    gosub_css3::media_query::set_media_environment(env);
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
    /// The device-pixel ratio and media preferences the page is laid out
    /// under (already applied process-wide by the caller); a retained page
    /// keeps them to lay out again under the same.
    pub dpr: u32,
    pub media: MediaPrefs,
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
/// parsing again - and, since the document stays with it, take the user's
/// input where the DOM is (see [`Self::input`]).
pub struct RetainedPage<C: RenderConfiguration> {
    title: Option<String>,
    favicon: Option<String>,
    /// The parsed document, typed: the input layer's setters and the
    /// hit-region walk need the concrete type, not the pipeline's view of it.
    doc: Arc<EngineDocument<C>>,
    /// The pipeline's view of the document, with the per-node style cache a
    /// re-layout keeps.
    adapter: Arc<GosubDocumentAdapter<C>>,
    base_url: Option<Url>,
    layer_list: Arc<LayerList>,
    layer_ids: Vec<LayerId>,
    fonts: Arc<Mutex<dyn FontSystem>>,
    media_store: Arc<gosub_render_pipeline::common::media::MediaStore>,
    rasterizer: Option<Box<dyn Rasterable + Send + Sync>>,
    viewport_width: f64,
    viewport_height: f64,
    page_width: f64,
    page_height: f64,
    /// The device-pixel ratio and media preferences the page was laid out
    /// under. Both are process-wide, and another tab's navigate in this
    /// renderer may have moved them since; a re-layout puts them back first.
    dpr: u32,
    media: MediaPrefs,
    /// The page's hit-test geometry, fixed at layout time.
    pub hit_regions: Vec<HitRegion>,
    /// Where the page's `#fragment` targets are, fixed at layout time.
    fragment_targets: Vec<crate::fork_server::protocol::FragmentTarget>,
    /// What the broker holds of this page, by tile position: the tiles a
    /// pass need not produce, and the pool eviction draws from.
    shipped: HashMap<TilePosKey, Shipped>,
    /// Where the viewport was last: a hover repaint stays within that window.
    scroll_y: f64,
    /// The DOM node under the pointer, as the broker last told us.
    hovered: Option<NodeId>,
    /// Gestures in progress and what the last one asked of the embedder.
    input: PageInput,
    /// What the input layer changed since the last pass.
    dirty: Dirty,
    /// Stage costs of `build`, reported with the first pass only.
    build_timings: Vec<(String, u64)>,
}

/// What an input pass has to redo: nothing, the tiles under some elements
/// (their margin boxes, unioned), or the layout.
enum Dirty {
    None,
    Paint(Rect),
    Relayout,
}

/// What one input pass produced: the tiles it changed, what it asked of the
/// broker, and whether the page was laid out again (its hit regions are new
/// then).
pub struct InputPass {
    pub pass: RenderPass,
    pub effects: Vec<Effect>,
    pub relayouted: bool,
}

/// Stages 1-3 from a styled document: render tree, layout, layering.
/// Returns the layer list and its ids, the page size, and the two stage
/// costs in microseconds.
fn lay_out<C: RenderConfiguration>(
    adapter: &Arc<GosubDocumentAdapter<C>>,
    fonts: &Arc<Mutex<dyn FontSystem>>,
    media_store: &Arc<gosub_render_pipeline::common::media::MediaStore>,
    viewport_width: f64,
    viewport_height: f64,
) -> (Arc<LayerList>, Vec<LayerId>, f64, f64, u64, u64) {
    use gosub_render_pipeline::layouter::taffy::TaffyLayouter;
    use gosub_render_pipeline::layouter::CanLayout;
    use gosub_render_pipeline::rendertree_builder::RenderTree;

    let started = std::time::Instant::now();
    let mut render_tree = RenderTree::new(Arc::clone(adapter) as Arc<dyn PipelineDocument>);
    if let Err(e) = render_tree.parse() {
        // Same degradation as the engine: the layouter tolerates a rootless
        // tree and the page renders empty.
        log::error!("failed to build render tree in the forked renderer: {e}");
    }
    let render_tree_us = started.elapsed().as_micros() as u64;

    // Layout, measured through the inherited font system. The media store
    // must be passed at construction: `with_font_system` builds a private
    // default store first, which is exactly the filesystem-touching
    // construction this process can no longer do.
    let mut layouter = TaffyLayouter::with_font_system_and_media_store(Arc::clone(fonts), Arc::clone(media_store));
    let vp_dim =
        (viewport_width > 0.0 && viewport_height > 0.0).then(|| Dimension::new(viewport_width, viewport_height));
    let layout_tree = layouter.layout(render_tree, vp_dim, 1.0);
    let page_width = layout_tree.root_dimension.width;
    let page_height = layout_tree.root_dimension.height;
    let layout_us = started.elapsed().as_micros() as u64 - render_tree_us;

    let layer_list = Arc::new(LayerList::new(Arc::new(layout_tree)));
    let layer_ids = layer_list.layer_ids.read().clone();
    (
        layer_list,
        layer_ids,
        page_width,
        page_height,
        render_tree_us,
        layout_us,
    )
}

/// Union of two rectangles.
fn union(a: Rect, b: Rect) -> Rect {
    let x0 = a.x.min(b.x);
    let y0 = a.y.min(b.y);
    let x1 = (a.x + a.width).max(b.x + b.width);
    let y1 = (a.y + a.height).max(b.y + b.height);
    Rect::new(x0, y0, x1 - x0, y1 - y0)
}

impl<C: RenderConfiguration> RetainedPage<C> {
    /// Stages 1-3 over `page`: parse (subresources through `loader`), apply
    /// hover, register web fonts, build the render tree, lay out, layer.
    pub fn build(
        page: PageRequest<'_>,
        fonts: Arc<Mutex<dyn FontSystem>>,
        media_store: Arc<gosub_render_pipeline::common::media::MediaStore>,
        loader: Arc<dyn ResourceLoader>,
    ) -> Self {
        let PageRequest {
            html,
            page_url,
            viewport_width,
            viewport_height,
            hovered_node,
            dpr,
            media,
            ..
        } = page;

        // Viewport-relative CSS units resolve against the real viewport; must
        // precede parse(), which computes styles for display:none filtering.
        gosub_css3::stylesheet::set_layout_viewport(viewport_width as f32, viewport_height as f32);

        // Parse with the page's base URL (relative subresource URLs resolve
        // against it). The parser records `<link rel="stylesheet">` and moves on;
        // the sheets are fetched through the broker once it is done and slotted
        // in where the parser found them - the engine's own arrangement, with the
        // renderer's loader in the fetcher's seat.
        let started = std::time::Instant::now();
        let base_url = Url::parse(page_url).ok();
        let mut stream = ByteStream::from_str(html, Encoding::UTF8);
        let mut doc = DocumentBuilderImpl::new_document::<C>(base_url.clone());
        let _ = Html5Parser::<C>::parse_document(&mut stream, &mut doc, Some(Html5ParserOptions::default()));
        doc.add_stylesheet(C::CssSystem::load_default_useragent_stylesheet());
        if doc.quirks_mode() == QuirksMode::Quirks {
            if let Some(quirks) = C::CssSystem::load_quirks_useragent_stylesheet() {
                doc.add_stylesheet(quirks);
            }
        }
        crate::engine::resource_pipeline::html::resolve_pending_stylesheets_blocking::<C>(&mut doc, &|url| {
            loader.fetch(url)
        });

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
            crate::engine::resource_pipeline::webfonts::load_web_fonts_blocking::<C>(
                &doc,
                base,
                &|url| loader.fetch(url),
                &mut |bytes, family| fonts.lock().register_font(bytes, Some(family)),
            );
        }
        let parse_us = started.elapsed().as_micros() as u64;
        let title = crate::html::document_title::<C>(&doc);
        let favicon = base_url
            .as_ref()
            .and_then(|base| crate::html::favicon_url::<C>(&doc, base))
            .map(|url| url.to_string());

        // Stages 1-3, from the document kept beside the pipeline's view of it.
        let doc = Arc::new(doc);
        let adapter = Arc::new(GosubDocumentAdapter::<C>::new(Arc::clone(&doc)));
        let (layer_list, layer_ids, page_width, page_height, render_tree_us, layout_us) =
            lay_out::<C>(&adapter, &fonts, &media_store, viewport_width, viewport_height);
        let hit_regions = collect_hit_regions::<C>(&layer_list, &doc, base_url.as_ref());
        let mut fragment_targets = crate::html::collect_fragment_targets(&layer_list, &doc);
        // The broker bounds what it keeps too; this bounds what crosses.
        fragment_targets.truncate(crate::fork_server::protocol::MAX_FRAGMENT_TARGETS);
        let build_timings = vec![
            ("build.parse".to_string(), parse_us),
            ("build.render_tree".to_string(), render_tree_us),
            ("build.layout".to_string(), layout_us),
        ];

        Self {
            title,
            favicon,
            doc,
            adapter,
            base_url,
            layer_list,
            layer_ids,
            rasterizer: C::forked_tile_rasterizer(Arc::clone(&fonts)),
            fonts,
            media_store,
            viewport_width,
            viewport_height,
            page_width,
            page_height,
            dpr,
            media,
            hit_regions,
            fragment_targets,
            shipped: HashMap::new(),
            scroll_y: 0.0,
            hovered,
            input: PageInput::default(),
            dirty: Dirty::None,
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

        // Only the two ancestor chains can gain or lose `:hover`.
        let mut dirty_nodes: Vec<NodeId> = Vec::new();
        let mut seen = HashSet::new();
        for start in [old_leaf, new_leaf].into_iter().flatten() {
            let mut id = start;
            loop {
                if seen.insert(id) {
                    dirty_nodes.push(id);
                }
                match self.doc.parent(id) {
                    Some(parent) => id = parent,
                    None => break,
                }
            }
        }
        self.doc.set_hovered_nodes(new_leaf);
        self.adapter.invalidate_style_for_nodes(&dirty_nodes);

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
                Some(u) => union(u, r),
            });
        }
        let Some(repaint) = repaint else {
            return self.empty_pass();
        };
        self.pass(Some(self.scroll_y), &HashSet::new(), Some(repaint))
    }

    /// The user acted on this page: apply `event` to the document through the
    /// input layer, then produce what it changed. Focus rings, carets and
    /// toggled controls that keep their box are a repaint of the tiles under
    /// them; anything that may move a box lays the page out again, after
    /// which the page ships by content hash against `known_tiles` (the
    /// broker's), as a navigate does - the tile bookkeeping by position does
    /// not survive a layout. What the input asked of the broker travels as
    /// effects.
    pub fn input(&mut self, scroll_y: f64, known_tiles: &HashSet<u64>, event: InputEvent) -> InputPass {
        self.scroll_y = scroll_y;
        self.dirty = Dirty::None;
        let focus_before = self.doc.focused_node();
        let capture_before = self.input.has_capture();
        let pointer = match &event {
            InputEvent::PointerDown { x, y, .. }
            | InputEvent::PointerUp { x, y, .. }
            | InputEvent::PointerMove { x, y }
            | InputEvent::Wheel { x, y, .. } => Some((*x, *y)),
            _ => None,
        };
        let mut effects = Vec::new();

        match event {
            InputEvent::PointerDown { x, y, button } => {
                if button == MouseButton::Left {
                    // Click-to-focus before any activation, as the tab worker orders it;
                    // a link under the pointer is the broker's to follow, not a control
                    // to activate.
                    self.with_input(|input, host| input.focus_at(host, x, y));
                    match self.link_under(x, y) {
                        Some(url) => effects.push(Effect::Navigate {
                            url,
                            post: false,
                            body: None,
                        }),
                        None => {
                            self.with_input(|input, host| input.activate_at(host, x, y));
                        }
                    }
                }
            }
            InputEvent::PointerUp { .. } => self.input.end_drag(),
            InputEvent::PointerMove { x, y } => {
                self.with_input(|input, host| {
                    PageInput::popup_hover_at(host, x, y);
                    input.drag_move(host, x, y)
                });
            }
            InputEvent::Wheel { x, y, delta_y } => {
                let _ = PageInput::popup_scroll(self, x, y, delta_y) || PageInput::area_scroll(self, x, y, delta_y);
            }
            InputEvent::KeyDown { key, modifiers } => {
                let modifiers = Modifiers::from_bits_truncate(modifiers);
                if let KeyOutcome::FollowLink(href) =
                    self.with_input(|input, host| input.key_down(host, &key, modifiers))
                {
                    if let Some(url) = self.resolve(&href) {
                        effects.push(Effect::Navigate {
                            url,
                            post: false,
                            body: None,
                        });
                    }
                }
            }
            InputEvent::KeyUp { .. } => {}
            InputEvent::Text { text } => {
                self.with_input(|input, host| input.insert_text(host, &text));
            }
            InputEvent::PickerChanged { value } => {
                self.with_input(|input, host| input.set_picker_value(host, &value));
            }
            InputEvent::PickerClosed => self.input.end_picker(),
            InputEvent::Blur => {
                self.with_input(|input, host| input.blur(host));
            }
        }

        // What the input asked for, in the order the tab worker collects it.
        if let Some(submission) = self.input.take_submission() {
            effects.push(Effect::Navigate {
                url: submission.url.to_string(),
                post: submission.post,
                body: submission.body,
            });
        }
        if let Some(request) = self.input.take_picker_request() {
            effects.push(Effect::Picker {
                kind: request.kind,
                bounds: wire_rect(request.anchor),
                value: request.value,
                min: request.min,
                max: request.max,
                step: request.step,
            });
        }
        if let Some(text) = self.input.take_clipboard_write() {
            effects.push(Effect::ClipboardWrite { text });
        }
        if self.input.take_paste_request() {
            effects.push(Effect::PasteRequested);
        }
        let focus_after = self.doc.focused_node();
        if focus_after != focus_before {
            effects.push(Effect::Focus {
                focused: focus_after.is_some(),
                editable: PageInput::focused_editable(self),
                bounds: focus_after.and_then(|node| self.control_bounds(node)),
            });
        }
        let capture = self.input.has_capture();
        if capture != capture_before {
            effects.push(Effect::Capture { pointer: capture });
        }
        if let Some((x, y)) = pointer {
            effects.push(Effect::Cursor {
                cursor: wire_cursor(self.input.cursor_at(self, x, y)),
            });
        }

        let (pass, relayouted) = match std::mem::replace(&mut self.dirty, Dirty::None) {
            Dirty::None => (self.empty_pass(), false),
            Dirty::Paint(rect) => (self.pass(Some(scroll_y), &HashSet::new(), Some(rect)), false),
            Dirty::Relayout => {
                let layout_us = self.lay_out_again();
                self.build_timings.push(("input.layout".to_string(), layout_us));
                // Positions and layer ids mean nothing across a layout: ship the
                // window again by content hash, and let go of what the broker
                // held that this pass did not account for.
                let held: HashSet<u64> = self.shipped.values().map(|s| s.hash).collect();
                self.shipped.clear();
                let mut pass = self.pass(Some(scroll_y), known_tiles, None);
                let now: HashSet<u64> = self.shipped.values().map(|s| s.hash).collect();
                pass.evicted.extend(held.difference(&now).copied());
                (pass, true)
            }
        };
        InputPass {
            pass,
            effects,
            relayouted,
        }
    }

    /// Stages 1-3 again over the retained document, after input changed what
    /// the boxes depend on. Every style is recomputed: a toggled control
    /// restyles its siblings through `:checked`, and which rules reach where
    /// is the cascade's to decide. Returns the cost in microseconds.
    fn lay_out_again(&mut self) -> u64 {
        let started = std::time::Instant::now();
        // Process-wide state another tab's navigate may have moved since.
        apply_media_prefs(self.dpr, self.media);
        gosub_css3::stylesheet::set_layout_viewport(self.viewport_width as f32, self.viewport_height as f32);
        self.adapter.clear_style_cache();
        let (layer_list, layer_ids, page_width, page_height, _, _) = lay_out::<C>(
            &self.adapter,
            &self.fonts,
            &self.media_store,
            self.viewport_width,
            self.viewport_height,
        );
        self.layer_list = layer_list;
        self.layer_ids = layer_ids;
        self.page_width = page_width;
        self.page_height = page_height;
        self.hit_regions = collect_hit_regions::<C>(&self.layer_list, &self.doc, self.base_url.as_ref());
        self.fragment_targets = crate::html::collect_fragment_targets(&self.layer_list, &self.doc);
        self.fragment_targets
            .truncate(crate::fork_server::protocol::MAX_FRAGMENT_TARGETS);
        started.elapsed().as_micros() as u64
    }

    /// The input layer, run against this page as its host. Taken out for the
    /// call, as the browsing context does: the host borrow is the whole page.
    fn with_input<R>(&mut self, f: impl FnOnce(&mut PageInput, &mut Self) -> R) -> R {
        let mut input = std::mem::take(&mut self.input);
        let out = f(&mut input, self);
        self.input = input;
        out
    }

    /// The link under a viewport point, resolved: the nearest `<a href>`
    /// enclosing the hit node, as a hit region would carry it.
    fn link_under(&self, vp_x: f64, vp_y: f64) -> Option<String> {
        let (node, _) = crate::engine::input::hit_at(Some(&self.layer_list), (0.0, self.scroll_y), vp_x, vp_y);
        describe_hit::<C>(&self.doc, node?, self.base_url.as_ref()).0
    }

    /// `href` resolved against the page, bounded like a hit region's link.
    fn resolve(&self, href: &str) -> Option<String> {
        self.base_url
            .as_ref()
            .and_then(|base| base.join(href).ok())
            .map(|url| url.to_string())
            .filter(|url| url.len() <= crate::fork_server::protocol::MAX_HIT_TEXT)
    }

    /// The border box of `node`'s layout element, in viewport CSS px.
    fn control_bounds(&self, node: NodeId) -> Option<WireRect> {
        let lei = crate::engine::input::layout_element_of(Some(&self.layer_list), node)?;
        let bb = self.layer_list.layout_tree.get_node_by_id(lei)?.box_model.border_box;
        Some(WireRect {
            x: bb.x,
            y: bb.y - self.scroll_y,
            width: bb.width,
            height: bb.height,
        })
    }

    /// What the hovered node's ancestry says: whether a link encloses it and
    /// the cursor for it, as a hit region would carry them.
    fn hover_facts(&self) -> (bool, CursorShape) {
        let Some(leaf) = self.hovered else {
            return (false, CursorShape::Default);
        };
        let (link, _, cursor, _) = describe_hit::<C>(&self.doc, leaf, self.base_url.as_ref());
        let cursor = match cursor {
            HitCursor::Pointer => CursorShape::Pointer,
            HitCursor::Text => CursorShape::Text,
            HitCursor::Resize => CursorShape::Resize,
            HitCursor::Default => CursorShape::Default,
        };
        (link.is_some(), cursor)
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
            fragment_targets: self.fragment_targets.clone(),
            no_page: false,
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
        // Across the layout width, not the viewport's: the tile grid takes its columns
        // from the layout, so a page that overflows sideways (or a zero-width viewport)
        // would otherwise leave every column past the viewport unpainted - the same
        // rule the in-process pipeline applies.
        let full_page_rect = Rect::new(
            0.0,
            0.0,
            self.page_width.max(self.viewport_width),
            self.page_height.max(1.0),
        );
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

impl<C: RenderConfiguration> InputHost for RetainedPage<C> {
    type Config = C;

    fn document(&self) -> Option<Arc<EngineDocument<C>>> {
        Some(Arc::clone(&self.doc))
    }

    fn layer_list(&self) -> Option<Arc<LayerList>> {
        Some(Arc::clone(&self.layer_list))
    }

    /// The wire carries no horizontal scroll; the page is laid out at the
    /// viewport's width.
    fn scroll(&self) -> (f64, f64) {
        (0.0, self.scroll_y)
    }

    fn viewport_height(&self) -> f64 {
        self.viewport_height
    }

    fn font_system(&self) -> Arc<Mutex<dyn FontSystem>> {
        Arc::clone(&self.fonts)
    }

    fn hover_has_link(&self) -> bool {
        self.hover_facts().0
    }

    fn hover_cursor(&self) -> CursorShape {
        self.hover_facts().1
    }

    fn repaint_elements(&mut self, elements: &[Option<LayoutElementId>]) {
        if matches!(self.dirty, Dirty::Relayout) {
            return;
        }
        for lei in elements.iter().flatten() {
            let Some(element) = self.layer_list.layout_tree.get_node_by_id(*lei) else {
                continue;
            };
            let m = &element.box_model.margin_box;
            let r = Rect::new(m.x, m.y, m.width, m.height);
            self.dirty = match std::mem::replace(&mut self.dirty, Dirty::None) {
                Dirty::Paint(u) => Dirty::Paint(union(u, r)),
                _ => Dirty::Paint(r),
            };
        }
    }

    fn damage_nodes(&mut self, nodes: &[NodeId]) {
        self.adapter.invalidate_style_for_nodes(nodes);
    }

    fn relayout(&mut self) {
        self.dirty = Dirty::Relayout;
    }
}

/// A pipeline rectangle in viewport coordinates, as an effect carries it.
fn wire_rect(r: Rect) -> WireRect {
    WireRect {
        x: r.x,
        y: r.y,
        width: r.width,
        height: r.height,
    }
}

fn wire_cursor(cursor: CursorShape) -> HitCursor {
    match cursor {
        CursorShape::Default => HitCursor::Default,
        CursorShape::Pointer => HitCursor::Pointer,
        CursorShape::Text => HitCursor::Text,
        CursorShape::Resize => HitCursor::Resize,
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
    use crate::fork_server::protocol::{HitCursor, MAX_HIT_TEXT};
    use gosub_interface::node::NodeType;
    // A URL past the bound is dropped, not cut: the broker navigates to it.
    let resolve = |raw: &str| {
        base_url
            .and_then(|b| b.join(raw).ok())
            .map(|u| u.to_string())
            .filter(|u| u.len() <= MAX_HIT_TEXT)
    };
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
                link = resolve(href);
                cursor = HitCursor::Pointer;
            }
        }
        if image.is_none() && doc.tag_name(current) == Some("img") {
            image = doc.attribute(current, "src").and_then(resolve);
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
    use crate::fork_server::protocol::MAX_HIT_TEXT_TOTAL;
    let mut regions = Vec::new();
    let layer_ids = layer_list.layer_ids.read();
    let layers = layer_list.layers.read();
    let mut text_bytes = 0usize;
    let mut text_dropped = false;

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
            let (mut link, mut image, cursor, editable) = describe_hit::<C>(doc, element.dom_node_id, base_url);
            // Past the page's text budget the region still hit-tests and
            // still says pointer; only the strings stay behind.
            text_bytes += link.as_ref().map_or(0, String::len) + image.as_ref().map_or(0, String::len);
            if text_bytes > MAX_HIT_TEXT_TOTAL {
                link = None;
                image = None;
                if !text_dropped {
                    text_dropped = true;
                    log::warn!("page carries more than {MAX_HIT_TEXT_TOTAL} bytes of link text; the rest ship without");
                }
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork_server::protocol::MediaPrefs;
    use gosub_css3::media_query::{media_environment, ColorScheme, ReducedMotion};

    /// What the broker sends reaches the environment `@media` reads; the
    /// viewport setter that follows keeps it. Light scheme, so the
    /// process-wide colour flags other tests read stay at their default.
    #[test]
    fn media_prefs_reach_the_media_environment() {
        apply_media_prefs(
            2,
            MediaPrefs {
                prefers_dark: false,
                prefers_reduced_motion: true,
            },
        );
        gosub_css3::stylesheet::set_layout_viewport(800.0, 600.0);
        let env = media_environment();
        assert_eq!(env.device_pixel_ratio, 2.0);
        assert_eq!(env.color_scheme, ColorScheme::Light);
        assert_eq!(env.reduced_motion, ReducedMotion::Reduce);
        assert_eq!((env.width, env.height), (800.0, 600.0));
        apply_media_prefs(1, MediaPrefs::default());
    }
}
