//! [`BrowsingContext`]: the runtime state for a single tab's document and rendering -
//! the parsed DOM, viewport, dirty-flag tracking, storage handles, and the pipeline
//! caches (tiles, render list, GPU scene) built from them.
//!
//! Loading itself lives in the tab worker; the worker hands a parsed document to the
//! context via `set_document`, after which the context rebuilds whichever render
//! representation the active backend consumes.

use crate::engine::events::{CursorShape, HitTestResponse};
use crate::engine::storage::{StorageArea, StorageHandles};
use crate::html::EngineDocument;
use gosub_config::{Config, HasConfig};
use gosub_render_pipeline::rasterizer::{
    collect_placed_gpu_tiles, cpu_cached_tiles, rasterize_parallel, rasterize_sequential, BakedTile, RasterStrategy,
    Rasterable, TilePixelCache,
};
use gosub_render_pipeline::render::{Color, DisplayItem, RenderContext, RenderList, Viewport};
use gosub_render_pipeline::tile_budget::TileBudget;
use std::sync::Arc;

use crate::html::RenderConfiguration;
use gosub_interface::css3::{CssSystem, HoverFingerprints};
use gosub_interface::document::Document as _;
use gosub_interface::node::NodeType;
use gosub_render_pipeline::common::browser_state::{BrowserState, WireframeState};
use gosub_render_pipeline::common::document::pipeline_doc::GosubDocumentAdapter;
use gosub_render_pipeline::common::geo::{Dimension as PipelineDimension, Rect as PipelineRect};
use gosub_render_pipeline::common::media::MediaStore;
use gosub_render_pipeline::common::texture::TilePixels;
use gosub_render_pipeline::layering::layer::{LayerId, LayerList};
use gosub_render_pipeline::layouter::taffy::TaffyLayouter;
use gosub_render_pipeline::layouter::{CanLayout, LayoutElementId};
use gosub_render_pipeline::painter::{PaintScene, Painter};
use gosub_render_pipeline::render::backend::{anchored_tile_pos, CachedTile, ExternalHandle};
use gosub_render_pipeline::rendertree_builder::RenderTree;
use gosub_render_pipeline::tile_budget::defer_tiles_outside_window;
use gosub_render_pipeline::tiler::{TileList, TileState};
use gosub_shared::node::NodeId;
use gosub_shared::{timing_start, timing_stop};
use std::any::Any;
use url::Url;

/// GPU-scene cache: the layer list (for hit-testing) plus the whole-page paint command list
/// (for the backend to render). The GPU equivalent of [`PipelineCache`] - it skips tiling,
/// rasterization, and tile compositing.
struct SceneCache {
    layer_list: Arc<LayerList>,
    scene: PaintScene,
}

/// True if `node_id` could be affected by a `:hover` rule, per the [`HoverFingerprints`]
/// computed by the CSS system. Uses only [`Document`] trait methods so it stays generic.
fn hover_matches<C: RenderConfiguration>(fp: &HoverFingerprints, doc: &EngineDocument<C>, node_id: NodeId) -> bool {
    if fp.has_universal {
        return true;
    }
    if let Some(tag) = doc.tag_name(node_id) {
        if fp.types.contains(tag) {
            return true;
        }
    }
    for cls in &fp.classes {
        if doc.has_class(node_id, cls) {
            return true;
        }
    }
    if !fp.ids.is_empty() {
        if let Some(id_attr) = doc.attribute(node_id, "id") {
            if fp.ids.contains(id_attr) {
                return true;
            }
        }
    }
    false
}
/// True for elements whose content is edited as text (they show the I-beam cursor).
fn is_text_input<C: RenderConfiguration>(doc: &EngineDocument<C>, node_id: NodeId) -> bool {
    match doc.tag_name(node_id) {
        Some("textarea") => true,
        Some("input") => !doc.attribute(node_id, "type").is_some_and(|t| {
            [
                "button", "submit", "reset", "checkbox", "radio", "range", "color", "file", "image", "hidden",
            ]
            .iter()
            .any(|k| t.eq_ignore_ascii_case(k))
        }),
        _ => doc
            .attribute(node_id, "contenteditable")
            .is_some_and(|v| v.is_empty() || v.eq_ignore_ascii_case("true")),
    }
}

/// True for elements that participate in keyboard focus: links, form controls,
/// editable regions, and anything with a non-negative `tabindex`.
fn is_focusable<C: RenderConfiguration>(doc: &EngineDocument<C>, node_id: NodeId) -> bool {
    if doc.node_type(node_id) != NodeType::ElementNode || doc.attribute(node_id, "disabled").is_some() {
        return false;
    }
    if let Some(tabindex) = doc.attribute(node_id, "tabindex") {
        return tabindex.trim().parse::<i32>().is_ok_and(|t| t >= 0);
    }
    match doc.tag_name(node_id) {
        Some("a" | "area") => doc.attribute(node_id, "href").is_some(),
        Some("input") => doc
            .attribute(node_id, "type")
            .is_none_or(|t| !t.eq_ignore_ascii_case("hidden")),
        Some("textarea" | "select" | "button") => true,
        _ => doc
            .attribute(node_id, "contenteditable")
            .is_some_and(|v| v.is_empty() || v.eq_ignore_ascii_case("true")),
    }
}

/// Cached output of stages 1–6 for the whole page. Re-used on every scroll tick.
struct PipelineCache {
    tiles: Vec<BakedTile>,
    page_height: f64,
    /// Pre-built CachedTile list (Arc-shared pixel data) for zero-copy scroll handles.
    cached_tiles: Arc<Vec<CachedTile>>,
    /// Layer list retained for hit-testing (hover).
    layer_list: Arc<LayerList>,
    /// Rasterized tile data keyed by (page_x, page_y, layer_id, content_hash).
    /// Passed to the next render so unchanged tiles skip rasterization.
    /// Value is (physical_width, physical_height, pixel_data).
    tile_pixel_cache: TilePixelCache,
}

/// BrowsingContext dedicated to a specific tab
///
/// A BrowsingContext is a single instance of the engine that deals with a specific tab. Each tab
/// has one BrowsingContext. These contexts though can use shared processes or threads, but not
/// from other contexts, only from the main engine.
pub struct BrowsingContext<C: RenderConfiguration = crate::html::DefaultRenderConfig> {
    /// Parsed DOM document
    document: Option<Arc<EngineDocument<C>>>,
    /// Storage handles for local and session storage
    storage: Option<StorageHandles>,

    // Rendering commands to paint the tab onto a surface
    render_list: RenderList,
    /// Render dirty flag, used to determine if the tab needs to be rendered
    render_dirty: bool,
    /// Viewport size (width/height only - scroll offset lives in scroll_x/y)
    viewport: Viewport,
    /// Epoch of the scene, used to determine if the scene has changed
    scene_epoch: u64,

    /// DOM dirty flag, used to determine if the DOM has changed
    dom_dirty: bool,
    /// Style dirty flag, used to determine if the styles have changed
    style_dirty: bool,
    /// Layout dirty flag, used to determine if the layout has changed
    layout_dirty: bool,

    /// Current scroll offset in CSS pixels.
    scroll_x: f64,
    scroll_y: f64,
    /// True when only the scroll offset changed (no full re-layout needed).
    scroll_dirty: bool,
    /// True when the scroll moved far enough that the raster window must be extended.
    /// Cheaper than `render_dirty`: extending re-uses the cached layout.
    raster_dirty: bool,

    /// Cached rasterized tiles for the full page. Valid until render_dirty is set.
    pipeline_cache: Option<PipelineCache>,
    /// GPU-scene cache (paint commands + layer list) for GPU backends. Mutually exclusive in
    /// practice with `pipeline_cache`: a tab uses one path or the other per its backend.
    scene_cache: Option<SceneCache>,
    /// Set when only hover state changed - triggers a paint-only repaint (stages 4–6),
    /// skipping the expensive render-tree rebuild (stage 1) and layout (stage 2).
    hover_dirty: bool,
    /// The DOM node currently under the pointer (for :hover matching).
    hover_leaf: Option<NodeId>,
    /// Layout element ID from the PREVIOUS hover update (needed to find which tile to repaint).
    hover_old_lei: Option<LayoutElementId>,
    /// DOM nodes whose hover state changed in the last update (old chain ∪ new chain).
    /// Only these nodes need their cached CSS invalidated; everything else in the tile stays cached.
    hover_dirty_nodes: Vec<NodeId>,
    /// The layout element currently under the pointer, used for bounding-box pre-check.
    hover_layout_element: Option<LayoutElementId>,
    /// Cached :hover fingerprints for the current document; rebuilt on document change.
    hover_fingerprints: Option<HoverFingerprints>,
    /// True when the last hover chain contained a fingerprint-sensitive node.
    hover_chain_sensitive: bool,
    /// The href of the link currently under the pointer, if any.
    pub hover_link_url: Option<String>,
    /// The focused element (mirrors the document's interior-mutable focus, which drives
    /// `:focus` matching; this field is the engine-side source of truth).
    focused_node: Option<NodeId>,
    /// Cursor shape for what is under the pointer, derived from the hovered node's ancestry.
    hover_cursor: CursorShape,

    /// The active backend's per-tile rasterizer and how to drive it. Built once by the tab
    /// worker from the engine's `RenderBackend` (replacing the former per-backend cfg cascade).
    rasterizer: Option<Box<dyn Rasterable + Send + Sync>>,
    raster_strategy: RasterStrategy,

    /// Media store shared between the layout and rasterization stages. The layouter loads
    /// images/SVGs into it by id; the rasterizer resolves the same ids back. It persists
    /// across renders so paint-only repaints (e.g. hover) still find previously loaded media.
    media_store: std::sync::Arc<MediaStore>,

    /// Per-engine settings store (cloned from the zone/engine). Read settings or subscribe to
    /// changes via [`HasConfig::config`].
    config_store: Config,

    /// LRU bookkeeping + eviction for the tile caches, bounded by the
    /// `renderer.tile.cache_budget_mb` setting.
    tile_budget: TileBudget,
}

impl<C: RenderConfiguration> BrowsingContext<C> {
    /// Creates a new runtime browsing context, sharing the given per-engine settings store.
    pub(crate) fn new(config_store: Config) -> BrowsingContext<C> {
        Self {
            document: None,
            storage: None,
            render_list: RenderList::new(),
            render_dirty: false,
            viewport: Viewport::default(),
            scene_epoch: 0,
            dom_dirty: false,
            style_dirty: false,
            layout_dirty: false,
            scroll_x: 0.0,
            scroll_y: 0.0,
            scroll_dirty: false,
            raster_dirty: false,
            pipeline_cache: None,
            scene_cache: None,
            hover_dirty: false,
            hover_leaf: None,
            hover_old_lei: None,
            hover_dirty_nodes: Vec::new(),
            hover_layout_element: None,
            hover_fingerprints: None,
            hover_chain_sensitive: false,
            hover_link_url: None,
            focused_node: None,
            hover_cursor: CursorShape::Default,
            rasterizer: None,
            raster_strategy: RasterStrategy::None,
            media_store: std::sync::Arc::new(MediaStore::new()),
            config_store,
            tile_budget: TileBudget::new(),
        }
    }

    /// True once the active backend's rasterizer has been installed (see [`Self::set_rasterizer`]).
    pub fn has_rasterizer(&self) -> bool {
        self.rasterizer.is_some()
    }

    /// Installs the active backend's per-tile rasterizer and raster strategy. Called once by the
    /// tab worker from `RenderBackend::create_rasterizer` / `raster_strategy`.
    pub fn set_rasterizer(&mut self, rasterizer: Box<dyn Rasterable + Send + Sync>, strategy: RasterStrategy) {
        self.rasterizer = Some(rasterizer);
        self.raster_strategy = strategy;
    }

    /// Binds the storage handles to the browsing context (@TODO: Why not via the ::new()?).
    pub fn bind_storage(&mut self, local: Arc<dyn StorageArea>, session: Arc<dyn StorageArea>) {
        self.storage = Some(StorageHandles { local, session });
    }
    pub fn local_storage(&self) -> Option<Arc<dyn StorageArea>> {
        self.storage.as_ref().map(|s| s.local.clone())
    }
    pub fn session_storage(&self) -> Option<Arc<dyn StorageArea>> {
        self.storage.as_ref().map(|s| s.session.clone())
    }

    /// Sets the parsed DOM document for the given tab.
    pub fn set_document(&mut self, doc: Arc<EngineDocument<C>>) {
        self.document = Some(doc);
        self.dom_dirty = true;
        self.style_dirty = true;
        self.layout_dirty = true;
        self.invalidate_render();
        self.pipeline_cache = None;
        self.scene_cache = None;
        self.tile_budget.reset();
        self.raster_dirty = false;
        self.hover_dirty = false;
        self.hover_leaf = None;
        self.hover_layout_element = None;
        self.hover_fingerprints = None;
        self.hover_chain_sensitive = false;
        self.hover_link_url = None;
        self.hover_cursor = CursorShape::Default;
        self.focused_node = None;
    }

    /// Update the viewport SIZE. Only triggers a full re-layout when width or height changes.
    /// Scroll offset is managed separately via `set_scroll`.
    pub fn set_viewport(&mut self, vp: Viewport) {
        if self.viewport.width == vp.width && self.viewport.height == vp.height {
            return;
        }
        self.viewport.width = vp.width;
        self.viewport.height = vp.height;
        self.layout_dirty = true;
        self.invalidate_render();
        self.pipeline_cache = None;
        self.scene_cache = None;
        self.tile_budget.reset();
        self.raster_dirty = false;
    }

    /// Update the scroll offset without triggering a full re-layout.
    /// The next composite will shift tiles by (x, y).
    pub fn set_scroll(&mut self, x: f64, y: f64) {
        let x = x.max(0.0);
        let max_y = self
            .active_page_height()
            .map(|ph| (ph - self.viewport.height as f64).max(0.0))
            .unwrap_or(f64::MAX);
        let y = y.max(0.0).min(max_y);
        if (self.scroll_x - x).abs() < 0.5 && (self.scroll_y - y).abs() < 0.5 {
            return;
        }
        self.scroll_x = x;
        self.scroll_y = y;
        self.scroll_dirty = true;
        // Compositing cannot conjure up tiles that were never rastered or were evicted, so ask
        // for a window extension rather than a full (re-laying-out) render.
        let page_height = self.active_page_height().unwrap_or(0.0);
        if self
            .tile_budget
            .needs_rerender(y, self.viewport.height as f64, page_height)
        {
            self.raster_dirty = true;
        }
    }

    /// Reset scroll to the top (called on navigation).
    pub fn reset_scroll(&mut self) {
        self.scroll_x = 0.0;
        self.scroll_y = 0.0;
    }

    #[inline]
    pub fn viewport(&self) -> &Viewport {
        &self.viewport
    }

    #[inline]
    pub fn scene_epoch(&self) -> u64 {
        self.scene_epoch
    }

    pub fn invalidate_render(&mut self) {
        self.render_dirty = true;
    }

    /// Poll whether a background media fetch (e.g. an image download started during layout) has
    /// completed since the last call. When it has, the cached layout is stale, so mark the render
    /// dirty and report `true` so the caller can also wake its own draw loop. The completion flag
    /// is consumed (cleared) by this call.
    pub fn poll_media_completed(&mut self) -> bool {
        if self.media_store.take_completed() {
            self.render_dirty = true;
            true
        } else {
            false
        }
    }

    /// Full pipeline rebuild (stages 1–6): re-tiles and re-rasterizes the whole page,
    /// carrying over the previous tile-pixel cache, then clears the content dirty flags.
    /// Shared by [`Self::rebuild_pipeline_cache_if_needed`] and
    /// [`Self::rebuild_render_list_if_needed`].
    fn rebuild_full_pipeline(&mut self) {
        if let Some(doc) = &self.document {
            let prev_tile_cache = self
                .pipeline_cache
                .as_mut()
                .map(|c| std::mem::take(&mut c.tile_pixel_cache))
                .unwrap_or_default();
            self.pipeline_cache = Some(pipeline_build_cache(
                doc.clone(),
                &self.viewport,
                self.scroll_y,
                self.rasterizer.as_deref(),
                self.raster_strategy,
                prev_tile_cache,
                self.media_store.clone(),
                self.config_store.get_uint("renderer.tile.size") as f64,
            ));
        }
        self.note_rastered_window();
        self.enforce_tile_budget(true);
        self.raster_dirty = false;
        self.render_dirty = false;
        self.hover_dirty = false;
        self.dom_dirty = false;
        self.style_dirty = false;
        self.layout_dirty = false;
    }

    /// Extend the raster window around the current scroll position, re-using the cached layout.
    /// Falls back to a full rebuild when there is no cache to extend.
    fn extend_raster_window(&mut self) {
        let Some(old_cache) = self.pipeline_cache.take() else {
            self.rebuild_full_pipeline();
            return;
        };
        let PipelineCache {
            layer_list,
            page_height,
            tile_pixel_cache,
            tiles,
            ..
        } = old_cache;

        self.pipeline_cache = Some(pipeline_extend_raster(
            layer_list,
            page_height,
            tiles,
            &self.viewport,
            self.scroll_y,
            self.rasterizer.as_deref(),
            self.raster_strategy,
            tile_pixel_cache,
            self.media_store.clone(),
            self.config_store.get_uint("renderer.tile.size") as f64,
        ));

        self.note_rastered_window();
        // Anything evicted that fell back inside the window has just been rastered again.
        self.tile_budget.note_full_raster();
        self.enforce_tile_budget(false);
        self.raster_dirty = false;
    }

    /// Record the window now rastered, so scrolling can tell when it reaches unbaked content.
    fn note_rastered_window(&self) {
        let Some(cache) = self.pipeline_cache.as_ref() else {
            return;
        };
        self.tile_budget
            .note_rastered_window(self.scroll_y, self.viewport.height as f64, cache.page_height);
    }

    /// Apply the `renderer.tile.cache_budget_mb` budget to the current pipeline cache, evicting
    /// LRU tiles outside the raster window. `full_raster` marks that every tile in the window was
    /// just re-rasterized, so previously evicted regions are live again.
    fn enforce_tile_budget(&mut self, full_raster: bool) {
        if full_raster {
            self.tile_budget.note_full_raster();
        }
        let Some(cache) = self.pipeline_cache.as_mut() else {
            return;
        };
        let budget_mb = self.config_store.get_uint("renderer.tile.cache_budget_mb");
        let report = self.tile_budget.enforce(
            &mut cache.tiles,
            &mut cache.tile_pixel_cache,
            self.scroll_y,
            self.viewport.height as f64,
            budget_mb.saturating_mul(1024 * 1024),
        );
        if report.evicted_tiles > 0 {
            // The compositor tile list must not keep evicted pixels alive; rebuild it.
            cache.cached_tiles = Arc::new(cpu_cached_tiles(&cache.tiles));
        }
    }

    /// Rebuild stages 1-6 (pipeline cache) if content has changed, without building a display
    /// list. Used by TileCache backends (Cairo, Skia, Vello) which composite tiles directly
    /// on the host thread and never consume the render list.
    ///
    /// Two paths:
    /// - **Full pipeline** (`render_dirty`): runs stages 1–6 for the whole page and caches
    ///   tiles. Triggered by navigation, DOM/style changes, or viewport resize.
    /// - **Paint-only repaint** (`hover_dirty`): reuses the cached layout tree and repaints
    ///   only the affected tiles, skipping stages 1–2.
    pub fn rebuild_pipeline_cache_if_needed(&mut self) {
        if !self.render_dirty && !self.hover_dirty && !self.scroll_dirty && !self.raster_dirty {
            return;
        }
        if self.render_dirty {
            self.rebuild_full_pipeline();
        } else if self.raster_dirty {
            self.extend_raster_window();
        } else if self.hover_dirty {
            // Paint-only repaint: reuse the cached layout tree, skip stages 1–2.
            if let Some(old_cache) = self.pipeline_cache.take() {
                let PipelineCache {
                    layer_list,
                    page_height,
                    tile_pixel_cache: prev_tile_cache,
                    tiles: prev_baked_tiles,
                    ..
                } = old_cache;
                self.pipeline_cache = Some(pipeline_hover_repaint(
                    layer_list,
                    page_height,
                    prev_baked_tiles,
                    self.hover_old_lei,
                    self.hover_layout_element,
                    &self.hover_dirty_nodes,
                    &self.viewport,
                    self.rasterizer.as_deref(),
                    self.raster_strategy,
                    prev_tile_cache,
                    self.media_store.clone(),
                    self.config_store.get_uint("renderer.tile.size") as f64,
                ));
                self.enforce_tile_budget(false);
            } else {
                // No cached layout yet - fall back to a full rebuild.
                if let Some(doc) = &self.document {
                    self.pipeline_cache = Some(pipeline_build_cache(
                        doc.clone(),
                        &self.viewport,
                        self.scroll_y,
                        self.rasterizer.as_deref(),
                        self.raster_strategy,
                        std::collections::HashMap::new(),
                        self.media_store.clone(),
                        self.config_store.get_uint("renderer.tile.size") as f64,
                    ));
                    self.note_rastered_window();
                    self.enforce_tile_budget(true);
                }
            }
            self.hover_dirty = false;
        }
        self.scroll_dirty = false;
        self.scene_epoch = self.scene_epoch.wrapping_add(1);
    }

    /// Build/refresh the device-agnostic render list if needed.
    ///
    /// Two paths:
    /// - **Full pipeline** (`render_dirty`): runs stages 1–6 for the whole page, caches tiles,
    ///   then composites. Triggered by navigation, DOM/style changes, or viewport resize.
    /// - **Scroll composite** (`scroll_dirty`): re-composites visible tiles from the cache with
    ///   the new scroll offset. No layout or rasterization work.
    pub fn rebuild_render_list_if_needed(&mut self) {
        if !self.render_dirty && !self.scroll_dirty && !self.raster_dirty {
            return;
        }

        if self.render_dirty {
            self.rebuild_full_pipeline();
        } else if self.raster_dirty {
            self.extend_raster_window();
        }

        let mut rl = RenderList::default();
        rl.items.push(DisplayItem::Clear {
            color: parse_clear_color(&self.config_store.get_string("renderer.clear_color")),
        });
        if let Some(cache) = &self.pipeline_cache {
            pipeline_composite(
                cache,
                self.scroll_x,
                self.scroll_y,
                self.viewport.width as f64,
                self.viewport.height as f64,
                &mut rl,
            );
            self.tile_budget.touch_composited(
                &cache.tiles,
                self.scroll_x,
                self.scroll_y,
                self.viewport.width as f64,
                self.viewport.height as f64,
            );
        }
        self.render_list = rl;

        self.scroll_dirty = false;
        self.scene_epoch = self.scene_epoch.wrapping_add(1);
    }

    /// GPU-scene path: rebuild the page's paint-command list when content changed.
    ///
    /// Runs stages 1–3 (render tree → layout → layering) and paints every element into one
    /// ordered command list - no tiling, rasterization, or tile compositing. Scroll-only changes
    /// don't rebuild anything (the backend re-renders with a new translate); they just advance the
    /// scene epoch so the worker emits a frame.
    pub fn rebuild_scene_cache_if_needed(&mut self) {
        if !self.render_dirty && !self.hover_dirty && !self.scroll_dirty {
            return;
        }
        // Both content changes and hover-style changes rebuild the command list. Hover could reuse
        // the cached layout (it only changes paint), but a GPU re-paint is cheap and avoids the
        // tile path's hover-repaint bookkeeping; revisit if hover proves hot.
        if self.render_dirty || self.hover_dirty {
            if let Some(doc) = &self.document {
                self.scene_cache = Some(pipeline_build_scene(
                    doc.clone(),
                    &self.viewport,
                    self.rasterizer.as_deref(),
                    self.media_store.clone(),
                ));
            }
            self.render_dirty = false;
            self.hover_dirty = false;
            self.dom_dirty = false;
            self.style_dirty = false;
            self.layout_dirty = false;
        }
        self.scroll_dirty = false;
        self.scene_epoch = self.scene_epoch.wrapping_add(1);
    }

    /// The active layer list for hit-testing - from the GPU scene cache or the CPU pipeline cache,
    /// whichever this tab's backend populates.
    fn active_layer_list(&self) -> Option<&Arc<LayerList>> {
        self.scene_cache
            .as_ref()
            .map(|c| &c.layer_list)
            .or_else(|| self.pipeline_cache.as_ref().map(|c| &c.layer_list))
    }

    /// Page-space top of the element a URL fragment points at, per the HTML "indicated part
    /// of the document": the element whose `id` equals the (percent-decoded) fragment, else
    /// the first `<a name=…>` with that name. An empty fragment or `top` means the top of the
    /// document. `None` when nothing matches or layout has not run yet.
    pub fn fragment_target_y(&self, fragment: &str) -> Option<f64> {
        let decoded = percent_encoding::percent_decode_str(fragment).decode_utf8_lossy();
        if decoded.is_empty() || decoded == "top" {
            return Some(0.0);
        }
        let doc = self.document.as_ref()?;
        let layer_list = self.active_layer_list()?;
        let arena = &layer_list.layout_tree.arena;
        let matches = |dom_id: NodeId, attr: &str| doc.attribute(dom_id, attr) == Some(decoded.as_ref());

        let by_id = arena.values().find(|n| matches(n.dom_node_id, "id"));
        let node = by_id.or_else(|| {
            arena
                .values()
                .find(|n| doc.tag_name(n.dom_node_id) == Some("a") && matches(n.dom_node_id, "name"))
        })?;
        Some(node.box_model.border_box.y)
    }

    /// Tile-cache statistics for diagnostics (`gosub://stats`): `(tile count, CPU pixel
    /// bytes)`. Bytes sum each baked tile's buffer, so shared buffers count once per tile
    /// (an upper bound, cheap to compute).
    pub fn tile_stats(&self) -> (usize, usize) {
        let Some(cache) = self.pipeline_cache.as_ref() else {
            return (0, 0);
        };
        let bytes = cache
            .tiles
            .iter()
            .map(|t| match &t.pixels {
                TilePixels::Cpu(data) => data.len(),
                _ => 0,
            })
            .sum();
        (cache.tiles.len(), bytes)
    }

    /// The active full-page height, from whichever cache this tab populates.
    fn active_page_height(&self) -> Option<f64> {
        self.scene_cache
            .as_ref()
            .map(|c| c.scene.page_height)
            .or_else(|| self.pipeline_cache.as_ref().map(|c| c.page_height))
    }

    /// If only the scroll offset changed (no content/layout change), returns a zero-copy
    /// `ExternalHandle::TileCache` that the host can composite directly, bypassing the Cairo
    /// render pipeline entirely. Returns `None` when a full render is needed.
    ///
    /// Calling this consumes the scroll-dirty flag and advances the scene epoch.
    pub fn take_scroll_handle(&mut self, dpr: u32) -> Option<ExternalHandle> {
        // With `raster_dirty` the cached tile list is missing tiles this frame needs.
        if !self.scroll_dirty || self.render_dirty || self.hover_dirty || self.raster_dirty {
            return None;
        }
        let cache = self.pipeline_cache.as_ref()?;
        let handle = ExternalHandle::TileCache {
            viewport_width: self.viewport.width,
            viewport_height: self.viewport.height,
            dpr,
            scroll_x: self.scroll_x as f32,
            scroll_y: self.scroll_y as f32,
            page_height: cache.page_height as f32,
            tiles: Arc::clone(&cache.cached_tiles),
        };
        self.tile_budget.touch_composited(
            &cache.tiles,
            self.scroll_x,
            self.scroll_y,
            self.viewport.width as f64,
            self.viewport.height as f64,
        );
        self.scroll_dirty = false;
        self.scene_epoch = self.scene_epoch.wrapping_add(1);
        Some(handle)
    }

    /// Returns a `TileCache` handle from the current pipeline cache regardless of dirty flags.
    /// Used by backends (e.g. Skia) that bypass the display-list render pipeline entirely
    /// and composite tiles directly on the host thread.
    pub fn tile_cache_handle(&self, dpr: u32) -> Option<ExternalHandle> {
        let cache = self.pipeline_cache.as_ref()?;
        self.tile_budget.touch_composited(
            &cache.tiles,
            self.scroll_x,
            self.scroll_y,
            self.viewport.width as f64,
            self.viewport.height as f64,
        );
        Some(ExternalHandle::TileCache {
            viewport_width: self.viewport.width,
            viewport_height: self.viewport.height,
            dpr,
            scroll_x: self.scroll_x as f32,
            scroll_y: self.scroll_y as f32,
            page_height: cache.page_height as f32,
            tiles: Arc::clone(&cache.cached_tiles),
        })
    }

    /// Returns the full page height from whichever cache is active (0 if not yet rendered).
    pub fn page_height(&self) -> f64 {
        self.active_page_height().unwrap_or(0.0)
    }

    /// Placed GPU tiles for the current pipeline cache, in page coordinates. Empty unless the
    /// active backend rasterized GPU-resident tiles. Handed to `RenderBackend::composite_tiles`.
    pub fn placed_gpu_tiles(&self) -> Vec<gosub_render_pipeline::render::backend::PlacedGpuTile> {
        self.pipeline_cache
            .as_ref()
            .map(|c| collect_placed_gpu_tiles(&c.tiles))
            .unwrap_or_default()
    }

    /// Current scroll offset in CSS pixels.
    pub fn scroll_xy(&self) -> (f64, f64) {
        (self.scroll_x, self.scroll_y)
    }

    /// Cursor shape for what is under the pointer, as of the last [`Self::update_hover`].
    pub fn hover_cursor(&self) -> CursorShape {
        self.hover_cursor
    }

    /// The currently focused element.
    pub fn focused_node(&self) -> Option<NodeId> {
        self.focused_node
    }

    /// Whether the focused element is text-editable (input/textarea/contenteditable).
    pub fn focused_editable(&self) -> bool {
        match (self.focused_node, self.document.as_ref()) {
            (Some(id), Some(doc)) => is_text_input(doc, id),
            _ => false,
        }
    }

    /// The focused element's link target (`<a href>`), for Enter-to-activate.
    pub fn focused_link(&self) -> Option<String> {
        let (id, doc) = (self.focused_node?, self.document.as_ref()?);
        if doc.tag_name(id) == Some("a") {
            doc.attribute(id, "href").map(str::to_string)
        } else {
            None
        }
    }

    /// Move focus to `node` (or clear it with `None`). Returns whether focus changed.
    /// A change re-styles the document so `:focus` rules apply.
    pub fn set_focus(&mut self, node: Option<NodeId>) -> bool {
        if self.focused_node == node {
            return false;
        }
        self.focused_node = node;
        if let Some(doc) = &self.document {
            doc.set_focused_node(node);
        }
        // Style-only change; the pipeline recomputes styles, layout and paint.
        self.style_dirty = true;
        self.invalidate_render();
        true
    }

    /// Focus the nearest focusable ancestor of the element at viewport point `(x, y)`
    /// (click-to-focus), blurring when the point hits nothing focusable. Returns whether
    /// focus changed.
    pub fn focus_at(&mut self, vp_x: f64, vp_y: f64) -> bool {
        let target = self.active_layer_list().and_then(|layer_list| {
            let lei = layer_list.find_element_at(vp_x, vp_y, self.scroll_x, self.scroll_y)?;
            layer_list.layout_tree.get_node_by_id(lei).map(|el| el.dom_node_id)
        });
        let focusable = target.and_then(|leaf| {
            let doc = self.document.as_ref()?;
            let mut id = leaf;
            loop {
                if is_focusable(doc.as_ref(), id) {
                    return Some(id);
                }
                match doc.parent(id) {
                    Some(parent) => id = parent,
                    None => return None,
                }
            }
        });
        self.set_focus(focusable)
    }

    /// Move focus to the next (or previous) focusable element in document order,
    /// wrapping around; from no focus, starts at the first (or last). Returns the newly
    /// focused element, or `None` when the document has none.
    pub fn focus_step(&mut self, backwards: bool) -> Option<NodeId> {
        let order = self.focusable_elements();
        if order.is_empty() {
            self.set_focus(None);
            return None;
        }
        let next = match self.focused_node.and_then(|cur| order.iter().position(|&n| n == cur)) {
            Some(pos) if backwards => order[(pos + order.len() - 1) % order.len()],
            Some(pos) => order[(pos + 1) % order.len()],
            None if backwards => *order.last()?,
            None => order[0],
        };
        self.set_focus(Some(next));
        Some(next)
    }

    /// Every focusable element, in document order.
    fn focusable_elements(&self) -> Vec<NodeId> {
        let Some(doc) = self.document.as_ref() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut stack = vec![doc.root()];
        while let Some(id) = stack.pop() {
            if is_focusable(doc.as_ref(), id) {
                out.push(id);
            }
            // Push children reversed so the stack yields document order.
            for &child in doc.children(id).iter().rev() {
                stack.push(child);
            }
        }
        out
    }

    /// Describe what is at viewport point `(vp_x, vp_y)` for a context menu: the nearest
    /// enclosing link, an image at the point, editable-ness, and the hit text node's
    /// content. URLs are resolved against `base`. Read-only: does not touch hover state.
    pub fn hit_test(&self, vp_x: f64, vp_y: f64, base: Option<&Url>) -> HitTestResponse {
        let mut out = HitTestResponse::default();
        let (Some(layer_list), Some(doc)) = (self.active_layer_list(), self.document.as_ref()) else {
            return out;
        };
        let Some(lei) = layer_list.find_element_at(vp_x, vp_y, self.scroll_x, self.scroll_y) else {
            return out;
        };
        let Some(leaf) = layer_list.layout_tree.get_node_by_id(lei).map(|el| el.dom_node_id) else {
            return out;
        };
        let resolve = |raw: &str| {
            base.and_then(|b| b.join(raw).ok())
                .map(|u| u.to_string())
                .unwrap_or_else(|| raw.to_string())
        };

        if doc.node_type(leaf) == NodeType::TextNode {
            out.text = doc
                .text_value(leaf)
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty());
        }
        let mut id = leaf;
        loop {
            match doc.tag_name(id) {
                Some("a") if out.link_url.is_none() => {
                    if let Some(href) = doc.attribute(id, "href") {
                        out.link_url = Some(resolve(href));
                    }
                }
                Some("img") if out.image_url.is_none() => {
                    if let Some(src) = doc.attribute(id, "src") {
                        out.image_url = Some(resolve(src));
                    }
                }
                _ => {}
            }
            if !out.is_editable && is_text_input(doc, id) {
                out.is_editable = true;
            }
            match doc.parent(id) {
                Some(parent) => id = parent,
                None => break,
            }
        }
        out
    }

    /// Hit-test at viewport coordinates `(vp_x, vp_y)` and update hover state.
    ///
    /// Returns `(visual_dirty, url_changed, link_url)`:
    /// - `visual_dirty`: a node with a `:hover` CSS rule entered or left the hover chain → needs repaint.
    /// - `url_changed`: the link URL under the cursor changed → caller should emit a `HoverUrl` event.
    /// - `link_url`: the href of the nearest `<a>` ancestor, if any.
    ///
    /// The cursor shape for the hovered node is derived in the same pass; read it with
    /// [`Self::hover_cursor`].
    pub fn update_hover(&mut self, vp_x: f64, vp_y: f64) -> (bool, bool, Option<String>) {
        let _t_total = gosub_shared::timing_guard!("hover.total");

        let (scroll_x, scroll_y) = (self.scroll_x, self.scroll_y);

        let (new_leaf, new_lei) = self.active_layer_list().map_or((None, None), |layer_list| {
            let _t = gosub_shared::timing_guard!("hover.hit_test");
            // find_element_at handles scroll per-layer (fixed layers ignore it).
            let Some(lei) = layer_list.find_element_at(vp_x, vp_y, scroll_x, scroll_y) else {
                return (None, None);
            };
            let dom_node_id = layer_list.layout_tree.get_node_by_id(lei).map(|el| el.dom_node_id);
            (dom_node_id, Some(lei))
        });

        // Common case: same element - skip the ancestor walk entirely.
        if new_leaf == self.hover_leaf {
            return (false, false, self.hover_link_url.clone());
        }

        self.hover_old_lei = self.hover_layout_element;

        // Collect old and new ancestor chains - only these nodes need CSS cache invalidation.
        self.hover_dirty_nodes.clear();
        if let Some(doc) = &self.document {
            let mut seen = std::collections::HashSet::new();
            for start in [self.hover_leaf, new_leaf].into_iter().flatten() {
                let mut id = start;
                loop {
                    if seen.insert(id) {
                        self.hover_dirty_nodes.push(id);
                    }
                    match doc.parent(id) {
                        Some(p) => id = p,
                        None => break,
                    }
                }
            }
        }

        self.hover_leaf = new_leaf;
        self.hover_layout_element = new_lei;

        // Build hover fingerprints lazily on first use after a document load.
        let fps = self.hover_fingerprints.get_or_insert_with(|| {
            self.document
                .as_ref()
                .map(|doc| <C::CssSystem as CssSystem>::hover_fingerprints(doc.stylesheets()))
                .unwrap_or_default()
        });

        // Walk the ancestor chain once for both link detection and fingerprint matching.
        // Terminate early once both are found.
        let (link_url, new_sensitive) = {
            let mut link: Option<String> = None;
            let mut sensitive = false;
            let mut cursor = CursorShape::Default;

            if let (Some(leaf), Some(doc)) = (new_leaf, self.document.as_ref()) {
                let _t = gosub_shared::timing_guard!("hover.ancestor_walk");
                // Text gets the I-beam unless an enclosing link (checked below) claims the
                // pointer hand.
                if doc.node_type(leaf) == NodeType::TextNode {
                    cursor = CursorShape::Text;
                }
                let mut id = leaf;
                loop {
                    if !sensitive && hover_matches(fps, doc, id) {
                        sensitive = true;
                    }
                    if link.is_none() && doc.tag_name(id) == Some("a") {
                        if let Some(href) = doc.attribute(id, "href") {
                            link = Some(href.to_string());
                            cursor = CursorShape::Pointer;
                        }
                    }
                    if cursor != CursorShape::Pointer && is_text_input(doc, id) {
                        cursor = CursorShape::Text;
                    }
                    if sensitive && link.is_some() {
                        break;
                    }
                    match doc.parent(id) {
                        Some(parent) => id = parent,
                        None => break,
                    }
                }
            }
            self.hover_cursor = cursor;
            (link, sensitive)
        };

        let url_changed = link_url != self.hover_link_url;
        self.hover_link_url = link_url.clone();

        // Only trigger a style recalc + repaint when a hover-sensitive node entered or left
        // the hover chain. If neither the old nor new chain touches a :hover rule, skip it.
        let visual_dirty = self.hover_chain_sensitive || new_sensitive;
        self.hover_chain_sensitive = new_sensitive;

        if visual_dirty {
            if let Some(doc) = &self.document {
                let _t = gosub_shared::timing_guard!("hover.set_hovered");
                doc.set_hovered_nodes(new_leaf);
            }
            // Hover-only changes are paint-only (color, background, box-shadow).
            // Use the cheap hover-dirty path which skips render-tree + layout.
            self.hover_dirty = true;
        }

        (visual_dirty, url_changed, link_url)
    }

    /// Returns the render list
    #[inline]
    pub fn render_list(&self) -> &RenderList {
        &self.render_list
    }
}

impl<C: RenderConfiguration> HasConfig for BrowsingContext<C> {
    fn config(&self) -> &Config {
        &self.config_store
    }
}

/// Parses a `#rrggbb` or `#rrggbbaa` hex color (the `renderer.clear_color` setting) into a
/// [`Color`]. Falls back to opaque white on any malformed input.
fn parse_clear_color(value: &str) -> Color {
    let hex = value.trim().trim_start_matches('#');
    let byte = |i: usize| hex.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok());

    match (byte(0), byte(2), byte(4)) {
        (Some(r), Some(g), Some(b)) => {
            let a = byte(6).unwrap_or(255);
            Color::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a as f32 / 255.0)
        }
        _ => Color::new(1.0, 1.0, 1.0, 1.0),
    }
}

impl<C: RenderConfiguration> RenderContext for BrowsingContext<C> {
    fn viewport(&self) -> &Viewport {
        &self.viewport
    }
    fn render_list(&self) -> &RenderList {
        &self.render_list
    }
    fn paint_scene(&self) -> Option<&dyn Any> {
        self.scene_cache.as_ref().map(|c| &c.scene as &dyn Any)
    }
    fn scroll_offset(&self) -> (f64, f64) {
        (self.scroll_x, self.scroll_y)
    }
}

/// GPU-scene build: stages 1–3 (render tree → layout → layering) plus a paint pass over every
/// element, producing one ordered paint-command list for the whole page. Skips tiling,
/// rasterization, and compositing - the backend renders the commands into a GPU texture.
fn pipeline_build_scene<C: RenderConfiguration>(
    doc: Arc<EngineDocument<C>>,
    viewport: &Viewport,
    rasterizer: Option<&(dyn Rasterable + Send + Sync)>,
    media_store: Arc<MediaStore>,
) -> SceneCache {
    // Resolve viewport-relative CSS units (vw/vh/vmin/vmax, incl. inside clamp()) against the
    // real viewport. Must precede parse(), which computes styles for display:none filtering.
    gosub_css3::stylesheet::set_layout_viewport(viewport.width as f32, viewport.height as f32);

    // Stage 1: render tree
    let adapter = GosubDocumentAdapter::<C>::new(doc);
    let mut render_tree = RenderTree::new(Arc::new(adapter));
    if let Err(e) = render_tree.parse() {
        log::error!("Failed to build render tree: {e}");
    }

    let vp_dim = if viewport.width > 0 && viewport.height > 0 {
        Some(PipelineDimension::new(viewport.width as f64, viewport.height as f64))
    } else {
        None
    };

    // Stage 2: layout (share the rasterizer's font system, as the tile path does)
    let mut layouter = match rasterizer.and_then(|r| r.font_system()) {
        Some(font_system) => TaffyLayouter::with_font_system(font_system),
        None => TaffyLayouter::new(),
    };
    layouter.set_media_store(Arc::clone(&media_store));
    let layout_tree = layouter.layout(render_tree, vp_dim, 1.0);
    let page_height = layout_tree.root_dimension.height;

    // Stage 3: layering
    let layer_list = Arc::new(LayerList::new(layout_tree));

    // Stage 5′: paint every element into one ordered list (no tiling). Paint over the full page
    // so scrolling reveals already-painted content without a rebuild.
    let layer_count = layer_list.layer_ids.read().len();
    let full_page_rect = PipelineRect::new(0.0, 0.0, viewport.width as f64, page_height.max(1.0));
    let state = BrowserState {
        visible_layer_list: vec![true; layer_count],
        wireframed: WireframeState::None,
        debug_hover: false,
        current_hovered_element: None,
        show_tilegrid: false,
        debug_table_cells: std::env::var("GOSUB_DEBUG_TABLE_CELLS").is_ok(),
        viewport: full_page_rect,
        tile_list: None,
        dpi_scale_factor: 1.0,
    };
    let painter = Painter::new(Arc::clone(&layer_list), rasterizer.and_then(|r| r.font_system()));
    let commands = painter.paint_all(&state);

    SceneCache {
        layer_list,
        scene: PaintScene {
            commands,
            media_store,
            page_height,
        },
    }
}

/// Runs pipeline stages 1–6 and returns a `PipelineCache` of rasterized tiles ready for repeated
/// compositing. Layout and tiling cover the whole page, but painting and rasterization cover only
/// the raster window around `scroll_y`, so first paint never pays for content nobody scrolls to.
///
/// Splitting the full pipeline from compositing lets scroll re-use the cached tiles without
/// re-running layout or rasterization.
#[allow(clippy::too_many_arguments)]
fn pipeline_build_cache<C: RenderConfiguration>(
    doc: Arc<EngineDocument<C>>,
    viewport: &Viewport,
    scroll_y: f64,
    rasterizer: Option<&(dyn Rasterable + Send + Sync)>,
    strategy: RasterStrategy,
    prev_tile_cache: TilePixelCache,
    media_store: Arc<MediaStore>,
    tile_size: f64,
) -> PipelineCache {
    let ts_total = timing_start!("pipeline.total");

    // Resolve viewport-relative CSS units (vw/vh/vmin/vmax, incl. inside clamp()) against the
    // real viewport. Must precede parse(), which computes styles for display:none filtering.
    gosub_css3::stylesheet::set_layout_viewport(viewport.width as f32, viewport.height as f32);

    // Stage 1: render tree
    let ts1 = timing_start!("pipeline.render_tree");
    let adapter = GosubDocumentAdapter::<C>::new(doc);
    let mut render_tree = RenderTree::new(Arc::new(adapter));
    if let Err(e) = render_tree.parse() {
        // The layouter tolerates a tree without a root; the frame degrades to empty.
        log::error!("Failed to build render tree: {e}");
    }
    timing_stop!(ts1);

    let vp_dim = if viewport.width > 0 && viewport.height > 0 {
        Some(PipelineDimension::new(viewport.width as f64, viewport.height as f64))
    } else {
        None
    };

    // Stage 2: layout
    let ts2 = timing_start!("pipeline.layout");
    // Share the rasterizer's font system so layout and rendering measure/draw against the
    // same font collection (and it's created once, not per layout pass). Backends without a
    // FontSystem (null, Cairo/Pango) fall back to the layouter's own instance.
    let mut layouter = match rasterizer.and_then(|r| r.font_system()) {
        Some(font_system) => TaffyLayouter::with_font_system(font_system),
        None => TaffyLayouter::new(),
    };
    // Share the persistent media store so resources loaded during layout are visible to the
    // rasterizer (which resolves them by id). Otherwise every image renders as a placeholder.
    layouter.set_media_store(Arc::clone(&media_store));
    let layout_tree = layouter.layout(render_tree, vp_dim, 1.0);
    timing_stop!(ts2);
    let page_height = layout_tree.root_dimension.height;

    // Stage 3: layering
    let ts3 = timing_start!("pipeline.layering");
    let layer_list = LayerList::new(layout_tree);
    timing_stop!(ts3);

    // Stage 4: tiling
    let ts4 = timing_start!("pipeline.tiling");
    let mut tile_list = TileList::new(layer_list, PipelineDimension::new(tile_size, tile_size));
    let saved_layer_list = Arc::clone(&tile_list.layer_list);
    tile_list.generate();
    timing_stop!(ts4);

    // Park the rest of the page: stages 5 and 6 below only touch dirty tiles.
    // Scrolling past the window's slack re-rasters around the new position.
    defer_tiles_outside_window(&mut tile_list, scroll_y, viewport.height as f64);

    let render_height = page_height;
    let ts5 = timing_start!("pipeline.painting");
    let full_page_rect = PipelineRect::new(0.0, 0.0, viewport.width as f64, render_height.max(1.0));
    let layer_ids = tile_list.layer_list.layer_ids.read().clone();
    paint_dirty_tiles(&mut tile_list, &layer_ids, full_page_rect, rasterizer);
    timing_stop!(ts5);

    // Stage 6: rasterize tiles using the active backend's rasterizer + strategy (chosen at
    // runtime by the engine's RenderBackend; no per-backend cfg here). Vello stays
    // sequential because all tiles share a Mutex<Renderer>; batching (not parallelism)
    // is the fix there.
    let (baked_tiles, new_tile_cache) = match (strategy, rasterizer) {
        (RasterStrategy::ParallelCached, Some(rasterizer)) => rasterize_parallel(
            rasterizer,
            &layer_ids,
            &mut tile_list,
            full_page_rect,
            &media_store,
            &prev_tile_cache,
            "pipeline.rasterize",
        ),
        (RasterStrategy::Sequential, Some(rasterizer)) => {
            rasterize_sequential(rasterizer, &layer_ids, &mut tile_list, full_page_rect, &media_store)
        }
        _ => (Vec::new(), std::collections::HashMap::new()),
    };

    timing_stop!(ts_total);

    // Pre-build the CachedTile list for zero-copy scroll handles.
    let cached_tiles = Arc::new(cpu_cached_tiles(&baked_tiles));

    PipelineCache {
        tiles: baked_tiles,
        page_height,
        cached_tiles,
        layer_list: saved_layer_list,
        tile_pixel_cache: new_tile_cache,
    }
}

/// Extend the raster window after a scroll: reuse the cached `LayerList` (so stages 1–2 are
/// skipped) and raster only the tiles that are newly inside the window. This is what keeps
/// scrolling a long page off the layout path.
#[allow(clippy::too_many_arguments)]
fn pipeline_extend_raster(
    layer_list: Arc<LayerList>,
    page_height: f64,
    prev_baked_tiles: Vec<BakedTile>,
    viewport: &Viewport,
    scroll_y: f64,
    rasterizer: Option<&(dyn Rasterable + Send + Sync)>,
    strategy: RasterStrategy,
    prev_tile_cache: TilePixelCache,
    media_store: Arc<MediaStore>,
    tile_size: f64,
) -> PipelineCache {
    // Stage 4: re-tile against the cached layout. No CSS, no layout.
    let ts4 = timing_start!("pipeline.extend.tiling");
    let mut tile_list = TileList::from_arc(Arc::clone(&layer_list), PipelineDimension::new(tile_size, tile_size));
    tile_list.generate();
    timing_stop!(ts4);

    // Already-baked tiles are carried over; the rest of the window is painted below.
    let mut prev_by_pos: std::collections::HashMap<(u64, u64, u64), BakedTile> = prev_baked_tiles
        .into_iter()
        .map(|t| ((t.page_x.to_bits(), t.page_y.to_bits(), t.layer_id), t))
        .collect();

    let mut clean_baked: Vec<BakedTile> = Vec::with_capacity(prev_by_pos.len());
    for tile in tile_list.arena.values_mut() {
        let key = (tile.rect.x.to_bits(), tile.rect.y.to_bits(), tile.layer_id.as_u64());
        let Some(baked) = prev_by_pos.remove(&key) else {
            continue;
        };
        tile.state = TileState::Ready;
        clean_baked.push(baked);
    }
    defer_tiles_outside_window(&mut tile_list, scroll_y, viewport.height as f64);

    let full_page_rect = PipelineRect::new(0.0, 0.0, viewport.width as f64, page_height.max(1.0));
    let layer_ids = tile_list.layer_list.layer_ids.read().clone();

    // Stage 5: only the newly in-window tiles are still dirty.
    let ts5 = timing_start!("pipeline.extend.painting");
    paint_dirty_tiles(&mut tile_list, &layer_ids, full_page_rect, rasterizer);
    timing_stop!(ts5);

    // Stage 6: the pixel cache makes tiles that were merely evicted cheap to bring back.
    let (baked_tiles, new_tile_cache) = match (strategy, rasterizer) {
        (RasterStrategy::ParallelCached, Some(rasterizer)) => rasterize_parallel(
            rasterizer,
            &layer_ids,
            &mut tile_list,
            full_page_rect,
            &media_store,
            &prev_tile_cache,
            "pipeline.extend.rasterize",
        ),
        (RasterStrategy::Sequential, Some(rasterizer)) => {
            rasterize_sequential(rasterizer, &layer_ids, &mut tile_list, full_page_rect, &media_store)
        }
        _ => (Vec::new(), std::collections::HashMap::new()),
    };

    // Keep carried-over entries: dropping them re-rasters those tiles on the next pass over.
    let mut merged_tile_cache = prev_tile_cache;
    merged_tile_cache.extend(new_tile_cache);

    let by_key: std::collections::HashMap<(u64, u64, u64), BakedTile> = baked_tiles
        .into_iter()
        .chain(clean_baked)
        .map(|t| ((t.page_x.to_bits(), t.page_y.to_bits(), t.layer_id), t))
        .collect();
    let all_baked_tiles = order_baked_tiles_by_layer(&tile_list, &layer_ids, full_page_rect, by_key);
    let cached_tiles = Arc::new(cpu_cached_tiles(&all_baked_tiles));

    PipelineCache {
        tiles: all_baked_tiles,
        page_height,
        cached_tiles,
        layer_list,
        tile_pixel_cache: merged_tile_cache,
    }
}

/// Hover-only repaint: skip stages 1–2 (render-tree + layout), reuse the cached
/// `LayerList`, and only repaint tiles that intersect the old or new hovered element.
/// All other tiles are carried over from `prev_baked_tiles` unchanged - no CSS
/// re-evaluation, no re-rasterization.
#[allow(clippy::too_many_arguments)]
fn pipeline_hover_repaint(
    layer_list: Arc<LayerList>,
    page_height: f64,
    prev_baked_tiles: Vec<BakedTile>,
    old_hover_lei: Option<LayoutElementId>,
    new_hover_lei: Option<LayoutElementId>,
    hover_dirty_nodes: &[NodeId],
    viewport: &Viewport,
    rasterizer: Option<&(dyn Rasterable + Send + Sync)>,
    strategy: RasterStrategy,
    prev_tile_cache: TilePixelCache,
    media_store: Arc<MediaStore>,
    tile_size: f64,
) -> PipelineCache {
    // Stage 4: tiling — reuse existing LayerList, no layout work.
    let ts4 = timing_start!("pipeline.hover.tiling");
    let mut tile_list = TileList::from_arc(Arc::clone(&layer_list), PipelineDimension::new(tile_size, tile_size));
    tile_list.generate();
    let total_tiles = tile_list.arena.len();
    timing_stop!(ts4);

    // Build a position-keyed lookup of previous baked tiles so non-hover tiles can be
    // carried over without any CSS re-evaluation or rasterization.
    // Key: (page_x bits, page_y bits, layer_id) - deterministic since tile positions don't
    // change. The layer id is essential: overlapping layers (e.g. the base layer and a sticky
    // header) share a page position, and keying by position alone would collapse them into one,
    // dropping the other tile and leaving a blank gap on the next hover repaint.
    let mut prev_by_pos: std::collections::HashMap<(u64, u64, u64), BakedTile> = prev_baked_tiles
        .into_iter()
        .map(|t| ((t.page_x.to_bits(), t.page_y.to_bits(), t.layer_id), t))
        .collect();

    // Compute the union bounding box of old and new hovered elements.  Tiles that
    // don't intersect this region cannot have changed visually, so we skip them.
    let hover_rect: Option<PipelineRect> = {
        let mut union: Option<PipelineRect> = None;
        for lei in [old_hover_lei, new_hover_lei].into_iter().flatten() {
            if let Some(el) = layer_list.layout_tree.get_node_by_id(lei) {
                let m = el.box_model.margin_box;
                let r = PipelineRect::new(m.x, m.y, m.width, m.height);
                union = Some(match union {
                    None => r,
                    Some(u) => {
                        let x0 = u.x.min(r.x);
                        let y0 = u.y.min(r.y);
                        let x1 = (u.x + u.width).max(r.x + r.width);
                        let y1 = (u.y + u.height).max(r.y + r.height);
                        PipelineRect::new(x0, y0, x1 - x0, y1 - y0)
                    }
                });
            }
        }
        union
    };

    // Full-page paint rect and back-to-front layer order - used both to re-emit carried tiles in
    // order (below / in the early-return) and by stages 5–6 further down.
    let full_page_rect = PipelineRect::new(0.0, 0.0, viewport.width as f64, page_height.max(1.0));
    let layer_ids = tile_list.layer_list.layer_ids.read().clone();

    // Mark tiles that DON'T intersect the hover region as Clean.  For Clean tiles we
    // carry the previous BakedTile forward; for Dirty tiles we re-evaluate CSS only
    // for the elements they contain (targeted invalidation).
    let mut clean_baked: Vec<BakedTile> = Vec::with_capacity(total_tiles);
    if let Some(hover_rect) = hover_rect {
        let doc = &layer_list.layout_tree.render_tree.doc;
        for tile in tile_list.arena.values_mut() {
            let tile_rect = tile.rect;
            let overlaps = tile_rect.x < hover_rect.x + hover_rect.width
                && tile_rect.x + tile_rect.width > hover_rect.x
                && tile_rect.y < hover_rect.y + hover_rect.height
                && tile_rect.y + tile_rect.height > hover_rect.y;
            if overlaps {
                // Invalidate cached styles only for the hover-chain nodes (old + new ancestors).
                // Non-hover elements in this tile keep their cached CSS - only the nodes that
                // actually gained or lost :hover need re-evaluation.
                doc.invalidate_style_for_nodes(hover_dirty_nodes);
                continue;
            }

            tile.state = TileState::Ready;
            let key = (tile_rect.x.to_bits(), tile_rect.y.to_bits(), tile.layer_id.as_u64());
            if let Some(baked) = prev_by_pos.remove(&key) {
                clean_baked.push(baked);
            }
        }
    } else {
        // No hover element visible - carry every previous tile forward, but re-emit in
        // back-to-front layer order (see order_baked_tiles_by_layer): `into_values()` is
        // unordered and would scramble overlapping-layer compositing.
        let all_tiles = order_baked_tiles_by_layer(&tile_list, &layer_ids, full_page_rect, prev_by_pos);
        let cached_tiles = Arc::new(cpu_cached_tiles(&all_tiles));
        return PipelineCache {
            tiles: all_tiles,
            page_height,
            cached_tiles,
            layer_list,
            tile_pixel_cache: prev_tile_cache,
        };
    }

    // Stage 5: paint ONLY dirty (hover-affected) tiles. `full_page_rect` and `layer_ids` were
    // computed above (shared with the carry-over ordering).
    let ts5 = timing_start!("pipeline.hover.painting");
    paint_dirty_tiles(&mut tile_list, &layer_ids, full_page_rect, rasterizer);
    timing_stop!(ts5);

    // Stage 6 (hover): rasterize the dirty tiles with the active backend's rasterizer + strategy.
    let (baked_tiles, new_tile_cache) = match (strategy, rasterizer) {
        (RasterStrategy::ParallelCached, Some(rasterizer)) => rasterize_parallel(
            rasterizer,
            &layer_ids,
            &mut tile_list,
            full_page_rect,
            &media_store,
            &prev_tile_cache,
            "pipeline.hover.rasterize",
        ),
        (RasterStrategy::Sequential, Some(rasterizer)) => {
            rasterize_sequential(rasterizer, &layer_ids, &mut tile_list, full_page_rect, &media_store)
        }
        _ => (Vec::new(), std::collections::HashMap::new()),
    };

    // Merge newly rasterized hover tiles + carried-over clean tiles, keyed by position+layer, then
    // re-emit in back-to-front layer order so overlapping layers composite correctly (a plain
    // `dirty ++ clean` concat scrambles the order - `clean_baked` came out of a HashMap - which
    // corrupts overlap regions like a sticky header and every scroll frame reusing this cache).
    let by_key: std::collections::HashMap<(u64, u64, u64), BakedTile> = baked_tiles
        .into_iter()
        .chain(clean_baked)
        .map(|t| ((t.page_x.to_bits(), t.page_y.to_bits(), t.layer_id), t))
        .collect();
    let all_baked_tiles = order_baked_tiles_by_layer(&tile_list, &layer_ids, full_page_rect, by_key);

    let cached_tiles = Arc::new(cpu_cached_tiles(&all_baked_tiles));

    PipelineCache {
        tiles: all_baked_tiles,
        page_height,
        cached_tiles,
        layer_list,
        tile_pixel_cache: new_tile_cache,
    }
}

/// Stage 5: paint every dirty tile. Callers steer the work through tile state - carried-over
/// (`Ready`) and out-of-window (`Deferred`) tiles are skipped.
fn paint_dirty_tiles(
    tile_list: &mut TileList,
    layer_ids: &[LayerId],
    full_page_rect: PipelineRect,
    rasterizer: Option<&(dyn Rasterable + Send + Sync)>,
) {
    let paint_state = BrowserState {
        visible_layer_list: vec![true; layer_ids.len()],
        wireframed: WireframeState::None,
        debug_hover: false,
        current_hovered_element: None,
        show_tilegrid: false,
        debug_table_cells: std::env::var("GOSUB_DEBUG_TABLE_CELLS").is_ok(),
        viewport: full_page_rect,
        tile_list: None,
        dpi_scale_factor: 1.0,
    };
    let painter = Painter::new(tile_list.layer_list.clone(), rasterizer.and_then(|r| r.font_system()));

    for &layer_id in layer_ids {
        for tile_id in tile_list.get_intersecting_tiles(layer_id, full_page_rect) {
            let Some(tile) = tile_list.get_tile_mut(tile_id) else {
                continue;
            };
            if tile.state != TileState::Dirty {
                continue;
            }
            for tiled_element in &mut tile.elements {
                tiled_element.paint_commands = painter.paint(tiled_element, &paint_state);
            }
        }
    }
}

/// Re-emit baked tiles in strict back-to-front layer order (the same order a full render
/// produces them). The compositor blits tiles in list order with source-over, so overlapping
/// layers (e.g. the base layer and a `position: sticky`/`fixed` header sharing a page position)
/// must stay layer-ordered or a lower tile paints over a higher one. `by_key` maps
/// `(page_x bits, page_y bits, layer_id)` → tile; positions with no baked tile (empty/transparent)
/// are simply skipped.
fn order_baked_tiles_by_layer(
    tile_list: &TileList,
    layer_ids: &[LayerId],
    full_page_rect: PipelineRect,
    mut by_key: std::collections::HashMap<(u64, u64, u64), BakedTile>,
) -> Vec<BakedTile> {
    let mut ordered = Vec::with_capacity(by_key.len());
    for &layer_id in layer_ids {
        for tile_id in tile_list.get_intersecting_tiles(layer_id, full_page_rect) {
            let Some(tile) = tile_list.arena.get(&tile_id) else {
                continue;
            };
            let key = (tile.rect.x.to_bits(), tile.rect.y.to_bits(), tile.layer_id.as_u64());
            if let Some(t) = by_key.remove(&key) {
                ordered.push(t);
            }
        }
    }
    ordered
}

/// Stage 7: composite visible tiles from the cache into `rl`.
///
/// Selects tiles that intersect `(scroll_x, scroll_y, vp_w, vp_h)` and blits them at
/// screen-relative positions. This is the only work done on every scroll tick.
fn pipeline_composite(cache: &PipelineCache, scroll_x: f64, scroll_y: f64, vp_w: f64, vp_h: f64, rl: &mut RenderList) {
    let ts7 = timing_start!("pipeline.composite");

    for tile in &cache.tiles {
        // Resolve the tile's position in viewport space (fixed tiles ignore scroll), then cull
        // against the viewport rect [0, vp].
        let (ex, ey) = anchored_tile_pos(tile.page_x, tile.page_y, scroll_x, scroll_y, tile.anchor);
        if ex + tile.width as f64 <= 0.0 || ey + tile.height as f64 <= 0.0 || ex >= vp_w || ey >= vp_h {
            continue;
        }

        // The display-list (null/CPU) compositor only handles CPU pixels; GPU-resident tiles are
        // composited by the backend's `composite_tiles` step instead.
        let TilePixels::Cpu(data) = &tile.pixels else {
            continue;
        };
        rl.items.push(DisplayItem::Blit {
            x: ex as f32,
            y: ey as f32,
            w: tile.width,
            h: tile.height,
            data: data.clone(),
            format: tile.format,
            opacity: tile.opacity,
        });
    }

    timing_stop!(ts7);
}

#[cfg(test)]
mod tests {
    use super::parse_clear_color;

    mod point_queries {
        use super::super::*;
        use crate::engine::settings_store;
        use crate::html::DefaultRenderConfig;
        use gosub_css3::system::Css3System;

        /// Lays out a page with an `id` target and an `<a name>` target at known offsets and
        /// resolves fragments against it.
        fn context_with_targets() -> BrowsingContext<DefaultRenderConfig> {
            let mut ctx: BrowsingContext<DefaultRenderConfig> = BrowsingContext::new(settings_store::default_config());
            ctx.set_viewport(Viewport {
                x: 0,
                y: 0,
                width: 400,
                height: 300,
            });
            let html = r#"<html><body style="margin:0">
                <div style="height:1000px"></div>
                <h2 id="section-2" style="margin:0;height:20px">Two</h2>
                <div style="height:500px"></div>
                <a name="legacy anchor" style="display:block;height:10px"></a>
                <div style="height:2000px"></div>
            </body></html>"#;
            let mut doc = gosub_html5::html_compile::<DefaultRenderConfig>(html);
            doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());
            ctx.set_document(Arc::new(doc));
            ctx.rebuild_pipeline_cache_if_needed();
            ctx
        }

        #[test]
        fn resolves_id_name_and_top() {
            let ctx = context_with_targets();
            // The h2 sits right after the 1000px spacer.
            let y = ctx.fragment_target_y("section-2").expect("id target");
            assert!((y - 1000.0).abs() < 1.0, "expected ~1000, got {y}");
            // `<a name>` fallback, percent-encoded in the URL: 1000 + 20 + 500.
            let y = ctx.fragment_target_y("legacy%20anchor").expect("name target");
            assert!((y - 1520.0).abs() < 1.0, "expected ~1520, got {y}");
            assert_eq!(ctx.fragment_target_y(""), Some(0.0));
            assert_eq!(ctx.fragment_target_y("top"), Some(0.0));
            assert_eq!(ctx.fragment_target_y("nope"), None);
        }

        /// Cursor shape derived from what is under the pointer: hand over links, I-beam over
        /// text and inputs, arrow elsewhere.
        #[test]
        fn hover_cursor_follows_content() {
            let mut ctx: BrowsingContext<DefaultRenderConfig> = BrowsingContext::new(settings_store::default_config());
            ctx.set_viewport(Viewport {
                x: 0,
                y: 0,
                width: 400,
                height: 400,
            });
            let html = r#"<html><body style="margin:0">
                <div style="height:100px;background:#eee"></div>
                <a href="/x" style="display:block;height:100px">link</a>
                <p style="margin:0;height:100px;font-size:20px">plain text</p>
                <input style="display:block;height:50px;width:200px">
            </body></html>"#;
            let mut doc = gosub_html5::html_compile::<DefaultRenderConfig>(html);
            doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());
            ctx.set_document(Arc::new(doc));
            ctx.rebuild_pipeline_cache_if_needed();

            // Empty div: arrow.
            ctx.update_hover(10.0, 50.0);
            assert_eq!(ctx.hover_cursor(), CursorShape::Default);
            // Link block (its text or its box): hand.
            ctx.update_hover(10.0, 150.0);
            assert_eq!(ctx.hover_cursor(), CursorShape::Pointer);
            // Text in the paragraph: I-beam.
            ctx.update_hover(10.0, 210.0);
            assert_eq!(ctx.hover_cursor(), CursorShape::Text);
            // Text input: I-beam.
            ctx.update_hover(10.0, 320.0);
            assert_eq!(ctx.hover_cursor(), CursorShape::Text);
        }

        /// Context-menu hit test: link/image/text/editable are independent facts about the
        /// point, URLs come back absolute.
        #[test]
        fn hit_test_describes_point() {
            let mut ctx: BrowsingContext<DefaultRenderConfig> = BrowsingContext::new(settings_store::default_config());
            ctx.set_viewport(Viewport {
                x: 0,
                y: 0,
                width: 400,
                height: 500,
            });
            let html = r#"<html><body style="margin:0">
                <div style="height:100px;background:#eee"></div>
                <a href="/target"><img src="pic.png" style="display:block;width:100px;height:100px"></a>
                <p style="margin:0;height:100px;font-size:20px">  some words  </p>
                <textarea style="display:block;height:50px;width:200px"></textarea>
            </body></html>"#;
            let mut doc = gosub_html5::html_compile::<DefaultRenderConfig>(html);
            doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());
            ctx.set_document(Arc::new(doc));
            ctx.rebuild_pipeline_cache_if_needed();
            let base = Url::parse("https://example.com/dir/page.html").unwrap();

            // Empty area.
            assert_eq!(ctx.hit_test(10.0, 50.0, Some(&base)), HitTestResponse::default());
            // Linked image: both facts, absolute URLs.
            let hit = ctx.hit_test(50.0, 150.0, Some(&base));
            assert_eq!(hit.link_url.as_deref(), Some("https://example.com/target"));
            assert_eq!(hit.image_url.as_deref(), Some("https://example.com/dir/pic.png"));
            assert!(!hit.is_editable);
            // Paragraph text: trimmed content, no link.
            let hit = ctx.hit_test(10.0, 210.0, Some(&base));
            assert_eq!(hit.text.as_deref(), Some("some words"));
            assert_eq!(hit.link_url, None);
            // Textarea: editable.
            let hit = ctx.hit_test(10.0, 320.0, Some(&base));
            assert!(hit.is_editable);
            // No document URL: raw attribute values pass through.
            let hit = ctx.hit_test(50.0, 150.0, None);
            assert_eq!(hit.link_url.as_deref(), Some("/target"));
        }

        /// Focus model: document-order traversal over focusable elements, wrap-around,
        /// click-to-focus via the nearest focusable ancestor, and `:focus` visibility
        /// through the document.
        #[test]
        fn focus_traversal_and_click_to_focus() {
            let mut ctx: BrowsingContext<DefaultRenderConfig> = BrowsingContext::new(settings_store::default_config());
            ctx.set_viewport(Viewport {
                x: 0,
                y: 0,
                width: 400,
                height: 500,
            });
            let html = r#"<html><body style="margin:0">
                <a href="/one" style="display:block;height:50px"><span>first</span></a>
                <div style="height:50px">not focusable</div>
                <input style="display:block;height:30px;width:200px">
                <a href="/two" tabindex="-1" style="display:block;height:30px">skipped</a>
                <button style="display:block;height:30px">go</button>
            </body></html>"#;
            let mut doc = gosub_html5::html_compile::<DefaultRenderConfig>(html);
            doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());
            ctx.set_document(Arc::new(doc));
            ctx.rebuild_pipeline_cache_if_needed();

            // Tab cycles a → input → button → wraps to a. The tabindex=-1 link is skipped.
            let a = ctx.focus_step(false).expect("first focusable");
            assert_eq!(ctx.focused_link().as_deref(), Some("/one"));
            let input = ctx.focus_step(false).expect("second");
            assert!(ctx.focused_editable());
            let button = ctx.focus_step(false).expect("third");
            assert!(!ctx.focused_editable());
            assert_eq!(ctx.focus_step(false), Some(a), "wraps around");
            assert_eq!(ctx.focus_step(true), Some(button), "shift-tab goes back");
            assert_ne!(a, input);

            // The document agrees (this is what :focus matching reads).
            let doc = ctx.document.as_ref().unwrap();
            assert!(doc.is_focused(button));
            assert!(!doc.is_focused(a));

            // Clicking the <span> inside the link focuses the link (nearest focusable
            // ancestor); clicking the plain div blurs.
            assert!(ctx.focus_at(10.0, 25.0));
            assert_eq!(ctx.focused_node(), Some(a));
            assert!(ctx.focus_at(10.0, 75.0));
            assert_eq!(ctx.focused_node(), None);
        }

        /// Regression: a layer whose elements sit entirely outside the page box (fixed
        /// element pushed far off-screen - a common accessibility/hiding pattern) used to
        /// underflow the tiler's tile-count arithmetic and panic the tab worker.
        #[test]
        fn offscreen_layer_does_not_panic_the_tiler() {
            let mut ctx: BrowsingContext<DefaultRenderConfig> = BrowsingContext::new(settings_store::default_config());
            ctx.set_viewport(Viewport {
                x: 0,
                y: 0,
                width: 400,
                height: 300,
            });
            let html = r#"<html><body style="margin:0">
                <div style="height:200px">content</div>
                <div style="position:fixed;left:5000px;top:8000px;width:50px;height:20px">off right+below</div>
                <div style="position:fixed;left:-9999px;top:10px;width:50px;height:20px">off left</div>
            </body></html>"#;
            let mut doc = gosub_html5::html_compile::<DefaultRenderConfig>(html);
            doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());
            ctx.set_document(Arc::new(doc));
            ctx.rebuild_pipeline_cache_if_needed();
            assert!(ctx.page_height() > 0.0, "page laid out");
        }

        #[test]
        fn unknown_before_layout() {
            let ctx: BrowsingContext<DefaultRenderConfig> = BrowsingContext::new(settings_store::default_config());
            assert_eq!(ctx.fragment_target_y("section-2"), None);
            // Top-of-document needs no layout.
            assert_eq!(ctx.fragment_target_y(""), Some(0.0));
        }
    }

    mod tile_budget_integration {
        use super::super::*;
        use crate::engine::settings_store;
        use crate::html::DefaultRenderConfig;
        use gosub_config::settings::Setting;
        use gosub_css3::system::Css3System;
        use gosub_render_pipeline::common::texture::TextureId;
        use gosub_render_pipeline::common::texture_store::TextureStore;
        use gosub_render_pipeline::render::backend::PixelFormat;
        use gosub_render_pipeline::tiler::Tile;
        use std::sync::atomic::{AtomicUsize, Ordering};

        /// Fills every tile with opaque pixels, so a baked tile costs exactly w * h * 4 bytes.
        /// The shared counter lets tests assert how many tiles a pass actually rasterized.
        struct SolidRasterizer {
            calls: Arc<AtomicUsize>,
        }

        impl Rasterable for SolidRasterizer {
            fn rasterize(&self, tile: &Tile, store: &mut TextureStore, _media: &MediaStore) -> Option<TextureId> {
                self.calls.fetch_add(1, Ordering::Relaxed);
                let (w, h) = (tile.rect.width as usize, tile.rect.height as usize);
                Some(store.add(w, h, vec![0xFFu8; w * h * 4], PixelFormat::PreMulArgb32))
            }
        }

        /// Unique pixel bytes held by the cache (baked tiles + pixel cache, shared buffers
        /// counted once) - the same accounting the budget enforces.
        fn resident_bytes(cache: &PipelineCache) -> usize {
            let mut counted = std::collections::HashSet::new();
            let mut total = 0usize;
            let buffers = cache
                .tiles
                .iter()
                .map(|t| &t.pixels)
                .chain(cache.tile_pixel_cache.values().map(|(_, _, p)| p));
            for pixels in buffers {
                if let TilePixels::Cpu(d) = pixels {
                    if counted.insert((d.as_ptr() as usize, d.len())) {
                        total += d.len();
                    }
                }
            }
            total
        }

        fn has_tile_near(cache: &PipelineCache, y: f64, within: f64) -> bool {
            cache.tiles.iter().any(|t| (t.page_y - y).abs() <= within)
        }

        const VP_H: u32 = 256;

        /// A context on a 10 000 px page: ~40 tile rows if fully rastered (~20 MiB), against a
        /// raster window three viewports tall.
        fn tall_page_context(budget_mb: usize) -> (BrowsingContext<DefaultRenderConfig>, Arc<AtomicUsize>) {
            let config = settings_store::default_config();
            assert!(config
                .set("renderer.tile.cache_budget_mb", Setting::UInt(budget_mb))
                .is_ok());

            let mut ctx: BrowsingContext<DefaultRenderConfig> = BrowsingContext::new(config);
            let calls = Arc::new(AtomicUsize::new(0));
            ctx.set_rasterizer(
                Box::new(SolidRasterizer {
                    calls: Arc::clone(&calls),
                }),
                RasterStrategy::ParallelCached,
            );
            ctx.set_viewport(Viewport {
                x: 0,
                y: 0,
                width: 512,
                height: VP_H,
            });

            let html =
                r#"<html><body style="margin:0"><div style="height:10000px;background:#ddd"></div></body></html>"#;
            let mut doc = gosub_html5::html_compile::<DefaultRenderConfig>(html);
            doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());
            ctx.set_document(Arc::new(doc));

            (ctx, calls)
        }

        #[test]
        fn first_render_rasterizes_only_the_window_of_a_tall_page() {
            let (mut ctx, raster_calls) = tall_page_context(128);

            ctx.rebuild_pipeline_cache_if_needed();

            let calls = raster_calls.swap(0, Ordering::Relaxed);
            let Some(cache) = ctx.pipeline_cache.as_ref() else {
                unreachable!("pipeline cache must exist after rebuild");
            };
            assert!(cache.page_height >= 10_000.0, "page must lay out tall");

            // The window at scroll 0 spans 2 tile rows; the whole page would be ~40.
            assert!(
                (1..=12).contains(&calls),
                "first paint rastered {calls} tiles, expected only the window"
            );
            assert!(has_tile_near(cache, 0.0, 0.5), "the viewport itself must be baked");
            assert!(
                !has_tile_near(cache, 5000.0, 256.0),
                "content far below the viewport must stay deferred"
            );
            assert_eq!(cache.cached_tiles.len(), cache.tiles.len());
        }

        #[test]
        fn scrolling_extends_the_window_without_relaying_out() {
            let (mut ctx, raster_calls) = tall_page_context(128);
            ctx.rebuild_pipeline_cache_if_needed();
            let first_pass = raster_calls.swap(0, Ordering::Relaxed);
            let page_height = ctx.page_height();

            // Inside the slack: composite-only.
            ctx.set_scroll(0.0, 50.0);
            assert!(!ctx.raster_dirty, "small scroll must not schedule rasterization");

            // Past it: an extension, not a full re-render.
            ctx.set_scroll(0.0, 5000.0);
            assert!(ctx.raster_dirty, "scrolling to unbaked content must raster");
            assert!(!ctx.render_dirty, "extending must not force a re-layout");
            assert!(
                ctx.take_scroll_handle(1).is_none(),
                "the composite-only path must not serve a frame with unbaked tiles"
            );

            ctx.rebuild_pipeline_cache_if_needed();

            let extend_pass = raster_calls.swap(0, Ordering::Relaxed);
            let Some(cache) = ctx.pipeline_cache.as_ref() else {
                unreachable!("pipeline cache must exist after extend");
            };
            assert!(has_tile_near(cache, 5000.0, 256.0), "the new viewport must be baked");
            assert!(
                extend_pass <= first_pass * 3,
                "extending rastered {extend_pass} tiles, expected a window"
            );
            assert_eq!(ctx.page_height(), page_height, "extending must reuse the cached layout");
            assert!(!ctx.raster_dirty, "the extension must satisfy the scroll");
        }

        #[test]
        fn cache_stays_under_budget_while_scrolling_the_whole_page() {
            const BUDGET_MB: usize = 4;
            const BUDGET_BYTES: usize = BUDGET_MB * 1024 * 1024;

            let (mut ctx, _calls) = tall_page_context(BUDGET_MB);
            ctx.rebuild_pipeline_cache_if_needed();

            // Doom-scroll the whole page in viewport-sized steps.
            let mut y = 0.0;
            while y < 10_000.0 {
                ctx.set_scroll(0.0, y);
                ctx.rebuild_pipeline_cache_if_needed();

                let Some(cache) = ctx.pipeline_cache.as_ref() else {
                    unreachable!("pipeline cache must exist while scrolling");
                };
                assert!(
                    resident_bytes(cache) <= BUDGET_BYTES,
                    "cache must stay within budget at scroll {y}: {} > {BUDGET_BYTES}",
                    resident_bytes(cache)
                );
                assert!(has_tile_near(cache, y, VP_H as f64), "viewport not baked at scroll {y}");
                y += VP_H as f64;
            }
        }
    }

    #[test]
    fn parse_clear_color_handles_rgb_rgba_and_garbage() {
        // 8-digit #rrggbbaa
        let c = parse_clear_color("#ff8000cc");
        assert!((c.r - 1.0).abs() < 1e-4);
        assert!((c.g - 0.5020).abs() < 1e-3);
        assert!((c.b - 0.0).abs() < 1e-4);
        assert!((c.a - 0.8).abs() < 1e-2);

        // 6-digit #rrggbb defaults alpha to opaque, leading '#' optional
        let c = parse_clear_color("00ff00");
        assert!((c.g - 1.0).abs() < 1e-4);
        assert!((c.a - 1.0).abs() < 1e-4);

        // Malformed input falls back to opaque white
        let c = parse_clear_color("not-a-color");
        assert_eq!((c.r, c.g, c.b, c.a), (1.0, 1.0, 1.0, 1.0));
    }
}
