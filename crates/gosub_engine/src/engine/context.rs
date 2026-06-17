//! Browsing context and tab runtime state.
//!
//! This module defines the [`BrowsingContext`] struct, which represents the runtime
//! state for a single tab, including its storage, rendering, and loading state. It
//! provides methods for loading URLs, binding storage, and managing the tab's state.
//!
//! # Overview
//!
//! The `BrowsingContext` is responsible for handling all aspects of a tab's state in
//! the browser engine. This includes managing the raw HTML content, the rendering
//! process, the viewport settings, and the storage for local and session data. It
//! also handles loading new content from URLs and updating the tab's state
//! accordingly.
//!
//! # Usage
//!
//! To use a `BrowsingContext`, you typically create a new instance, configure it as
//! needed (e.g., set the viewport, bind storage), and then load a URL. After loading,
//! you can access the rendered content and other state information. The context also
//! provides mechanisms to handle navigation events, such as redirects or loading
//! errors.
//!
//! # Example
//!
//! ```no_run
//! ```
//!
//! # Structs
//!
//! - [`BrowsingContext`]: The main struct representing the browsing context for a tab.
//!
//! # Errors
//!
//! - [`LoadError`]: Represents errors that can occur while loading content, such as
//!   navigation cancellations or network errors.

use crate::engine::storage::{StorageArea, StorageHandles};
use crate::html::EngineDocument;
use gosub_config::{Config, HasConfig};
use gosub_render_pipeline::rasterizer::{RasterStrategy, Rasterable};
use gosub_render_pipeline::render::{Color, DisplayItem, RenderContext, RenderList, Viewport};
use std::sync::Arc;
use url::Url;

use crate::html::RenderConfiguration;
use gosub_interface::css3::{CssSystem, HoverFingerprints};
use gosub_interface::document::Document as _;
use gosub_render_pipeline::layering::layer::LayerList;
use gosub_render_pipeline::layouter::LayoutElementId;
use gosub_render_pipeline::painter::{PaintScene, Painter};
use gosub_render_pipeline::render::backend::{CachedTile, ExternalHandle};
use gosub_shared::node::NodeId;
use std::any::Any;

/// GPU-scene cache: the layer list (for hit-testing) plus the whole-page paint command list
/// (for the backend to render). The GPU equivalent of [`PipelineCache`] — it skips tiling,
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
// #[derive(Debug, thiserror::Error)]
// pub enum LoadError {
//     #[error("navigation cancelled")]
//     Cancelled,
//     #[error(transparent)]
//     Net(#[from] reqwest::Error),
// }

/// A single rasterized tile with its page-coordinate position, ready to blit.
struct BakedTile {
    page_x: f64,
    page_y: f64,
    width: u32,
    height: u32,
    data: bytes::Bytes,
    /// In-memory byte order of `data`, set by the rasterizer that produced it.
    format: gosub_render_pipeline::render::backend::PixelFormat,
}

/// Key that uniquely identifies a tile's content for cache lookup.
/// Format: (page_x bits, page_y bits, layer_id, paint-command hash).
type TileCacheKey = (u64, u64, u64, u64);

/// Rasterized tile cache: maps a [`TileCacheKey`] to `(physical_width, physical_height, pixels)`.
/// Carried between renders so unchanged tiles skip rasterization.
type TilePixelCache = std::collections::HashMap<TileCacheKey, (u32, u32, bytes::Bytes)>;

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
    // /// Is there anything that needs to be rendered or redrawn?
    // dirty: DirtyFlags,
    /// Current URL being processed
    current_url: Option<Url>,
    /// Parsed DOM document
    document: Option<Arc<EngineDocument<C>>>,
    /// True when the tab has failed loading (mostly net issues)
    failed: bool,

    // Tokio runtime for async operations
    // runtime: Arc<Runtime>,
    /// Storage handles for local and session storage
    storage: Option<StorageHandles>,

    // Rendering commands to paint the tab onto a surface
    render_list: RenderList,
    /// Render dirty flag, used to determine if the tab needs to be rendered
    render_dirty: bool,
    /// Viewport size (width/height only — scroll offset lives in scroll_x/y)
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

    /// Cached rasterized tiles for the full page. Valid until render_dirty is set.
    pipeline_cache: Option<PipelineCache>,
    /// GPU-scene cache (paint commands + layer list) for GPU backends. Mutually exclusive in
    /// practice with `pipeline_cache`: a tab uses one path or the other per its backend.
    scene_cache: Option<SceneCache>,
    /// Set when only hover state changed — triggers a paint-only repaint (stages 4–6),
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

    /// The active backend's per-tile rasterizer and how to drive it. Built once by the tab
    /// worker from the engine's `RenderBackend` (replacing the former per-backend cfg cascade).
    rasterizer: Option<Box<dyn Rasterable + Send + Sync>>,
    raster_strategy: RasterStrategy,

    /// Media store shared between the layout and rasterization stages. The layouter loads
    /// images/SVGs into it by id; the rasterizer resolves the same ids back. It persists
    /// across renders so paint-only repaints (e.g. hover) still find previously loaded media.
    media_store: std::sync::Arc<gosub_render_pipeline::common::media::MediaStore>,

    /// Per-engine settings store (cloned from the zone/engine). Read settings or subscribe to
    /// changes via [`HasConfig::config`].
    config_store: Config,
}

impl<C: RenderConfiguration> BrowsingContext<C> {
    /// Creates a new runtime browsing context, sharing the given per-engine settings store.
    pub(crate) fn new(config_store: Config) -> BrowsingContext<C> {
        Self {
            current_url: None,
            document: None,
            failed: false,
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
            rasterizer: None,
            raster_strategy: RasterStrategy::None,
            media_store: std::sync::Arc::new(gosub_render_pipeline::common::media::MediaStore::new()),
            config_store,
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
        {
            self.pipeline_cache = None;
            self.scene_cache = None;
            self.hover_dirty = false;
            self.hover_leaf = None;
            self.hover_layout_element = None;
            self.hover_fingerprints = None;
            self.hover_chain_sensitive = false;
        }
    }

    /// Update the viewport SIZE. Only triggers a full re-layout when width or height changes.
    /// Scroll offset is managed separately via `set_scroll`.
    pub fn set_viewport(&mut self, vp: Viewport) {
        if self.viewport.width != vp.width || self.viewport.height != vp.height {
            self.viewport.width = vp.width;
            self.viewport.height = vp.height;
            self.layout_dirty = true;
            self.invalidate_render();
            {
                self.pipeline_cache = None;
                self.scene_cache = None;
            }
        }
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

    /// Build/refresh the device-agnostic scene if needed.
    ///
    /// Two paths:
    /// - **Full pipeline** (`render_dirty`): runs stages 1–6 for the whole page, caches tiles,
    ///   then composites. Triggered by navigation, DOM/style changes, or viewport resize.
    ///
    /// Rebuild stages 1-6 (pipeline cache) if content has changed, without building a display
    /// list. Used by TileCache backends (Cairo, Skia, Vello) which composite tiles directly
    /// on the host thread and never consume the render list.
    pub fn rebuild_pipeline_cache_if_needed(&mut self) {
        if !self.render_dirty && !self.hover_dirty && !self.scroll_dirty {
            return;
        }
        if self.render_dirty {
            if let Some(doc) = &self.document {
                let prev_tile_cache = self
                    .pipeline_cache
                    .as_mut()
                    .map(|c| std::mem::take(&mut c.tile_pixel_cache))
                    .unwrap_or_default();
                self.pipeline_cache = Some(pipeline_build_cache(
                    doc.clone(),
                    &self.viewport,
                    self.rasterizer.as_deref(),
                    self.raster_strategy,
                    prev_tile_cache,
                    self.media_store.clone(),
                ));
            }
            self.render_dirty = false;
            self.hover_dirty = false;
            self.dom_dirty = false;
            self.style_dirty = false;
            self.layout_dirty = false;
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
                ));
            } else {
                // No cached layout yet — fall back to a full rebuild.
                if let Some(doc) = &self.document {
                    self.pipeline_cache = Some(pipeline_build_cache(
                        doc.clone(),
                        &self.viewport,
                        self.rasterizer.as_deref(),
                        self.raster_strategy,
                        std::collections::HashMap::new(),
                        self.media_store.clone(),
                    ));
                }
            }
            self.hover_dirty = false;
        }
        self.scroll_dirty = false;
        self.scene_epoch = self.scene_epoch.wrapping_add(1);
    }

    /// - **Scroll composite** (`scroll_dirty`): re-composites visible tiles from the cache with
    ///   the new scroll offset. No layout or rasterization work.
    pub fn rebuild_render_list_if_needed(&mut self) {
        if !self.render_dirty && !self.scroll_dirty {
            return;
        }

        {
            if self.render_dirty {
                if let Some(doc) = &self.document {
                    let prev_tile_cache = self
                        .pipeline_cache
                        .as_mut()
                        .map(|c| std::mem::take(&mut c.tile_pixel_cache))
                        .unwrap_or_default();
                    self.pipeline_cache = Some(pipeline_build_cache(
                        doc.clone(),
                        &self.viewport,
                        self.rasterizer.as_deref(),
                        self.raster_strategy,
                        prev_tile_cache,
                        self.media_store.clone(),
                    ));
                }
                self.render_dirty = false;
                self.hover_dirty = false;
                self.dom_dirty = false;
                self.style_dirty = false;
                self.layout_dirty = false;
            }

            let mut rl = RenderList::default();
            rl.items.push(DisplayItem::Clear {
                color: Color::new(1.0, 1.0, 1.0, 1.0),
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
            }
            self.render_list = rl;
        }

        self.scroll_dirty = false;
        self.scene_epoch = self.scene_epoch.wrapping_add(1);
    }

    /// GPU-scene path: rebuild the page's paint-command list when content changed.
    ///
    /// Runs stages 1–3 (render tree → layout → layering) and paints every element into one
    /// ordered command list — no tiling, rasterization, or tile compositing. Scroll-only changes
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

    /// The active layer list for hit-testing — from the GPU scene cache or the CPU pipeline cache,
    /// whichever this tab's backend populates.
    fn active_layer_list(&self) -> Option<&Arc<LayerList>> {
        self.scene_cache
            .as_ref()
            .map(|c| &c.layer_list)
            .or_else(|| self.pipeline_cache.as_ref().map(|c| &c.layer_list))
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
        if !self.scroll_dirty || self.render_dirty || self.hover_dirty {
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
        self.scroll_dirty = false;
        self.scene_epoch = self.scene_epoch.wrapping_add(1);
        Some(handle)
    }

    /// Returns a `TileCache` handle from the current pipeline cache regardless of dirty flags.
    /// Used by backends (e.g. Skia) that bypass the display-list render pipeline entirely
    /// and composite tiles directly on the host thread.
    pub fn tile_cache_handle(&self, dpr: u32) -> Option<ExternalHandle> {
        let cache = self.pipeline_cache.as_ref()?;
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

    /// Hit-test at viewport coordinates `(vp_x, vp_y)` and update hover state.
    ///
    /// Returns `(visual_dirty, url_changed, link_url)`:
    /// - `visual_dirty`: a node with a `:hover` CSS rule entered or left the hover chain → needs repaint.
    /// - `url_changed`: the link URL under the cursor changed → caller should emit a `HoverUrl` event.
    /// - `link_url`: the href of the nearest `<a>` ancestor, if any.
    pub fn update_hover(&mut self, vp_x: f64, vp_y: f64) -> (bool, bool, Option<String>) {
        let _t_total = gosub_shared::timing_guard!("hover.total");

        let page_x = vp_x + self.scroll_x;
        let page_y = vp_y + self.scroll_y;

        let (new_leaf, new_lei) = self.active_layer_list().map_or((None, None), |layer_list| {
            let _t = gosub_shared::timing_guard!("hover.hit_test");
            let Some(lei) = layer_list.find_element_at(page_x, page_y) else {
                return (None, None);
            };
            let dom_node_id = layer_list.layout_tree.get_node_by_id(lei).map(|el| el.dom_node_id);
            (dom_node_id, Some(lei))
        });

        // Common case: same element — skip the ancestor walk entirely.
        if new_leaf == self.hover_leaf {
            return (false, false, self.hover_link_url.clone());
        }

        self.hover_old_lei = self.hover_layout_element;

        // Collect old and new ancestor chains — only these nodes need CSS cache invalidation.
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

            if let (Some(leaf), Some(doc)) = (new_leaf, self.document.as_ref()) {
                let _t = gosub_shared::timing_guard!("hover.ancestor_walk");
                let mut id = leaf;
                loop {
                    if !sensitive && hover_matches(fps, doc, id) {
                        sensitive = true;
                    }
                    if link.is_none() && doc.tag_name(id) == Some("a") {
                        if let Some(href) = doc.attribute(id, "href") {
                            link = Some(href.to_string());
                        }
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

    /// Returns true when the loading failed
    pub fn has_failed(&self) -> bool {
        self.failed
    }

    /// Returns the current loaded the tab (or None when nothing has loaded yet)
    pub fn current_url(&self) -> Option<&Url> {
        self.current_url.as_ref()
    }
}

impl<C: RenderConfiguration> HasConfig for BrowsingContext<C> {
    fn config(&self) -> &Config {
        &self.config_store
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

/// Runs pipeline stages 1–6 for the **entire page** (all tiles, not just the viewport slice)
/// Compute a stable cache key for a tile: (page_x bits, page_y bits, layer_id, content hash).
/// The content hash covers all paint commands so any visual change produces a different key.
fn tile_cache_key(tile: &gosub_render_pipeline::tiler::Tile) -> TileCacheKey {
    use gosub_render_pipeline::painter::commands::{
        border::{BorderRadius, BorderStyle},
        brush::Brush,
        PaintCommand,
    };

    // Minimal inline FNV-1a hasher — no trait bounds needed on the types being hashed.
    let mut h: u64 = 14695981039346656037;
    macro_rules! fnv {
        ($bytes:expr) => {
            for b in $bytes {
                h ^= *b as u64;
                h = h.wrapping_mul(1099511628211);
            }
        };
    }
    macro_rules! hf32 {
        ($v:expr) => {
            fnv!(&$v.to_bits().to_le_bytes())
        };
    }
    macro_rules! hf64 {
        ($v:expr) => {
            fnv!(&$v.to_bits().to_le_bytes())
        };
    }
    macro_rules! hu64 {
        ($v:expr) => {
            fnv!(&($v as u64).to_le_bytes())
        };
    }
    macro_rules! hbool {
        ($v:expr) => {
            fnv!(&[$v as u8])
        };
    }
    macro_rules! hstr {
        ($s:expr) => {
            fnv!($s.as_bytes());
            fnv!(&[0u8])
        };
    }

    macro_rules! hash_brush {
        ($b:expr) => {
            match $b {
                Brush::Solid(c) => {
                    fnv!(&[0]);
                    hf32!(c.r());
                    hf32!(c.g());
                    hf32!(c.b());
                    hf32!(c.a());
                }
                Brush::Image(m) => {
                    fnv!(&[1]);
                    hu64!(m.as_u64());
                }
            }
        };
    }

    // Hash tile background.
    match tile.bgcolor {
        Some((r, g, b, a)) => {
            hbool!(true);
            hf32!(r);
            hf32!(g);
            hf32!(b);
            hf32!(a);
        }
        None => hbool!(false),
    }

    for elem in &tile.elements {
        hu64!(elem.id.as_u64());
        hf64!(elem.rect.x);
        hf64!(elem.rect.y);
        hf64!(elem.rect.width);
        hf64!(elem.rect.height);

        for cmd in &elem.paint_commands {
            match cmd {
                PaintCommand::Rectangle(r) => {
                    fnv!(&[0u8]);
                    let rect = r.rect();
                    hf64!(rect.x);
                    hf64!(rect.y);
                    hf64!(rect.width);
                    hf64!(rect.height);
                    match r.background() {
                        None => hbool!(false),
                        Some(b) => {
                            hbool!(true);
                            hash_brush!(b);
                        }
                    }
                    let border = r.border();
                    hf32!(border.width());
                    fnv!(&[match border.style() {
                        BorderStyle::Solid => 1,
                        BorderStyle::Dashed => 2,
                        BorderStyle::Dotted => 3,
                        BorderStyle::Double => 4,
                        BorderStyle::Groove => 5,
                        BorderStyle::Ridge => 6,
                        BorderStyle::Inset => 7,
                        BorderStyle::Outset => 8,
                        BorderStyle::Hidden => 9,
                        BorderStyle::None => 0,
                    }]);
                    for b in border.brushes() {
                        hash_brush!(&b);
                    }
                    if let Some(tr) = border.radius() {
                        hbool!(true);
                        for br in [&tr.top, &tr.right, &tr.bottom, &tr.left] {
                            match br {
                                BorderRadius::Uniform(v) => {
                                    fnv!(&[0]);
                                    hf32!(*v);
                                }
                                BorderRadius::Elliptical { horizontal, vertical } => {
                                    fnv!(&[1]);
                                    hf32!(*horizontal);
                                    hf32!(*vertical);
                                }
                            }
                        }
                    } else {
                        hbool!(false);
                    }
                    let (tl, tr, br, bl) = r.radius_x();
                    hf64!(tl);
                    hf64!(tr);
                    hf64!(br);
                    hf64!(bl);
                    let (tl, tr, br, bl) = r.radius_y();
                    hf64!(tl);
                    hf64!(tr);
                    hf64!(br);
                    hf64!(bl);
                }
                PaintCommand::Text(t) => {
                    fnv!(&[1u8]);
                    hf64!(t.rect.x);
                    hf64!(t.rect.y);
                    hf64!(t.rect.width);
                    hf64!(t.rect.height);
                    hstr!(&t.text);
                    hstr!(&t.font_info.family);
                    hf64!(t.font_info.size);
                    hf64!(t.font_info.line_height);
                    hu64!(t.font_info.weight as u64);
                    hu64!(t.font_info.width as u64);
                    hu64!(t.font_info.slant as u64);
                    hbool!(t.font_info.underline);
                    hbool!(t.font_info.line_through);
                    hash_brush!(&t.brush);
                }
                PaintCommand::Svg(s) => {
                    fnv!(&[2u8]);
                    hu64!(s.media_id.as_u64());
                    let rect = s.rect.rect();
                    hf64!(rect.x);
                    hf64!(rect.y);
                    hf64!(rect.width);
                    hf64!(rect.height);
                }
            }
        }
    }

    (tile.rect.x.to_bits(), tile.rect.y.to_bits(), tile.layer_id.as_u64(), h)
}

/// Sequential per-tile rasterization, used by GPU backends (e.g. Vello) whose shared
/// `Mutex<Renderer>` rules out parallelism. No dirty-tile cache, so it returns an empty one.
fn rasterize_sequential(
    rasterizer: &(dyn Rasterable + Send + Sync),
    layer_ids: &[gosub_render_pipeline::layering::layer::LayerId],
    tile_list: &mut gosub_render_pipeline::tiler::TileList,
    full_page_rect: gosub_render_pipeline::common::geo::Rect,
    media_store: &gosub_render_pipeline::common::media::MediaStore,
) -> (Vec<BakedTile>, TilePixelCache) {
    use gosub_render_pipeline::common::texture_store::TextureStore;
    use gosub_render_pipeline::tiler::TileState;
    use gosub_shared::{timing_start, timing_stop};

    let ts6 = timing_start!("pipeline.rasterize");
    let mut texture_store = TextureStore::new();

    for &layer_id in layer_ids {
        let tile_ids = tile_list.get_intersecting_tiles(layer_id, full_page_rect);
        for tile_id in tile_ids {
            if let Some(tile) = tile_list.get_tile_mut(tile_id) {
                if tile.state == TileState::Dirty {
                    match rasterizer.rasterize(tile, &mut texture_store, media_store) {
                        Some(texture_id) => {
                            tile.texture_id = Some(texture_id);
                            tile.state = TileState::Clean;
                        }
                        None => tile.state = TileState::Empty,
                    }
                }
            }
        }
    }

    let mut tiles: Vec<BakedTile> = Vec::with_capacity(tile_list.arena.len());
    for tile in tile_list.arena.values() {
        if let (Some(texture_id), true) = (tile.texture_id, tile.state == TileState::Clean) {
            if let Some(tex) = texture_store.get(texture_id) {
                tiles.push(BakedTile {
                    page_x: tile.rect.x,
                    page_y: tile.rect.y,
                    width: tex.width as u32,
                    height: tex.height as u32,
                    data: tex.data.clone(),
                    format: tex.format,
                });
            }
        }
    }

    timing_stop!(ts6);
    (tiles, std::collections::HashMap::new())
}

/// and returns a `PipelineCache` of rasterized tiles ready for repeated compositing.
///
/// Splitting the full pipeline from compositing lets scroll re-use the cached tiles without
/// re-running layout or rasterization.
/// GPU-scene build: stages 1–3 (render tree → layout → layering) plus a paint pass over every
/// element, producing one ordered paint-command list for the whole page. Skips tiling,
/// rasterization, and compositing — the backend renders the commands into a GPU texture.
fn pipeline_build_scene<C: RenderConfiguration>(
    doc: Arc<EngineDocument<C>>,
    viewport: &Viewport,
    rasterizer: Option<&(dyn Rasterable + Send + Sync)>,
    media_store: Arc<gosub_render_pipeline::common::media::MediaStore>,
) -> SceneCache {
    use gosub_render_pipeline::common::browser_state::{BrowserState, WireframeState};
    use gosub_render_pipeline::common::document::pipeline_doc::GosubDocumentAdapter;
    use gosub_render_pipeline::common::geo::{Dimension as PipelineDimension, Rect as PipelineRect};
    use gosub_render_pipeline::layouter::taffy::TaffyLayouter;
    use gosub_render_pipeline::layouter::CanLayout;
    use gosub_render_pipeline::rendertree_builder::RenderTree;

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
    let painter = Painter::new(Arc::clone(&layer_list));
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

fn pipeline_build_cache<C: RenderConfiguration>(
    doc: Arc<EngineDocument<C>>,
    viewport: &Viewport,
    rasterizer: Option<&(dyn Rasterable + Send + Sync)>,
    strategy: RasterStrategy,
    prev_tile_cache: TilePixelCache,
    media_store: Arc<gosub_render_pipeline::common::media::MediaStore>,
) -> PipelineCache {
    use gosub_render_pipeline::common::browser_state::{BrowserState, WireframeState};
    use gosub_render_pipeline::common::document::pipeline_doc::GosubDocumentAdapter;
    use gosub_render_pipeline::common::geo::{Dimension as PipelineDimension, Rect as PipelineRect};
    use gosub_render_pipeline::layering::layer::LayerList;
    use gosub_render_pipeline::layouter::taffy::TaffyLayouter;
    use gosub_render_pipeline::layouter::CanLayout;
    use gosub_render_pipeline::painter::Painter;
    use gosub_render_pipeline::rendertree_builder::RenderTree;
    use gosub_render_pipeline::tiler::{TileList, TileState};
    use gosub_shared::{timing_start, timing_stop};

    let ts_total = timing_start!("pipeline.total");

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
    let mut tile_list = TileList::new(layer_list, PipelineDimension::new(256.0, 256.0));
    let saved_layer_list = Arc::clone(&tile_list.layer_list);
    tile_list.generate();
    timing_stop!(ts4);

    // Stage 5: paint all tiles for the full page so that scrolling reveals pre-rendered
    // content. We use the full page_height rather than capping to viewport.height; the
    // compositor only ships the visible subset to the screen anyway, so no extra pixels
    // are transferred. Memory is bounded by tile count: at 256×256×4B per tile, a 6 000 px
    // page × 1 280 px wide = ~120 tiles × 256 KB each ≈ 30 MB, which is acceptable.
    let render_height = page_height;
    let ts5 = timing_start!("pipeline.painting");
    let full_page_rect = PipelineRect::new(0.0, 0.0, viewport.width as f64, render_height.max(1.0));
    let layer_ids = tile_list.layer_list.layer_ids.read().clone();
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
    let painter = Painter::new(tile_list.layer_list.clone());
    for &layer_id in &layer_ids {
        let tile_ids = tile_list.get_intersecting_tiles(layer_id, full_page_rect);
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
    timing_stop!(ts5);

    // Stage 6: rasterize ALL tiles → collect into BakedTile vec.
    //
    // Cairo and Skia rasterize tiles independently (no shared GPU state), so we use
    // rayon to spread the work across all available CPU cores.  Each worker gets its
    // own temporary TextureStore so there is zero shared mutable state in the hot loop.
    //
    // Three phases:
    //   1. Collect IDs of dirty tiles (sequential — cheap iteration).
    //   2. Rasterize each tile in parallel → Option<BakedTile>.
    //   3. Update tile states and gather results (sequential — trivial).
    //
    // Vello stays sequential because all tiles share a Mutex<Renderer>; batching
    // (not parallelism) is the fix there.
    macro_rules! rasterize_parallel {
        ($rasterizer:expr) => {{
            use gosub_render_pipeline::common::texture_store::TextureStore;
            use gosub_render_pipeline::render::backend::PixelFormat;
            use gosub_render_pipeline::tiler::TileId;
            use rayon::prelude::*;

            // Cairo and Skia both emit premultiplied ARGB32 (BGRA byte order).
            let tile_format = PixelFormat::PreMulArgb32;

            let ts6 = timing_start!("pipeline.rasterize");
            // `media_store` (the shared store populated during layout) comes from the enclosing
            // function; rasterize() resolves image/SVG ids against it. &Arc derefs to &MediaStore.
            let rasterizer = $rasterizer;

            // Phase 1: collect IDs of dirty tiles across all layers.
            let dirty_ids: Vec<TileId> = layer_ids
                .iter()
                .flat_map(|&layer_id| tile_list.get_intersecting_tiles(layer_id, full_page_rect))
                .filter(|&id| tile_list.arena.get(&id).map_or(false, |t| t.state == TileState::Dirty))
                .collect();

            // Phase 2: parallel rasterization with dirty-tile cache.
            // For each tile: compute a content hash; if it matches the previous render's cached
            // pixels, reuse them (cache hit). Otherwise rasterize on this thread.
            // Result: (tile_id, Option<BakedTile>, Option<new_cache_entry>)
            type CacheEntry = (TileCacheKey, (u32, u32, bytes::Bytes));
            let results: Vec<(TileId, Option<BakedTile>, Option<CacheEntry>)> = dirty_ids
                .par_iter()
                .map(|&tile_id| {
                    let Some(tile) = tile_list.arena.get(&tile_id) else {
                        return (tile_id, None, None);
                    };

                    let key = tile_cache_key(tile);

                    // Cache hit: same content as the previous render — reuse pixels.
                    if let Some(&(w, h, ref data)) = prev_tile_cache.get(&key) {
                        let baked = BakedTile {
                            page_x: tile.rect.x,
                            page_y: tile.rect.y,
                            width: w,
                            height: h,
                            data: data.clone(),
                            format: tile_format,
                        };
                        return (tile_id, Some(baked), None);
                    }

                    // Cache miss: rasterize and emit a new cache entry.
                    let mut local_store = TextureStore::new();
                    let baked = rasterizer
                        .rasterize(tile, &mut local_store, &media_store)
                        .and_then(|tid| local_store.get(tid))
                        .map(|tex| BakedTile {
                            page_x: tile.rect.x,
                            page_y: tile.rect.y,
                            width: tex.width as u32,
                            height: tex.height as u32,
                            data: tex.data.clone(),
                            format: tex.format,
                        });

                    let cache_entry = baked.as_ref().map(|b| (key, (b.width, b.height, b.data.clone())));
                    (tile_id, baked, cache_entry)
                })
                .collect();

            // Phase 3: update tile states, gather BakedTiles, and build the new tile cache.
            let mut tiles: Vec<BakedTile> = Vec::with_capacity(results.len());
            let mut new_tile_cache: TilePixelCache = std::collections::HashMap::with_capacity(results.len());

            for (tile_id, baked, cache_entry) in results {
                if let Some(tile) = tile_list.arena.get_mut(&tile_id) {
                    match baked {
                        Some(b) => {
                            tile.state = TileState::Clean;
                            if let Some(entry) = cache_entry {
                                new_tile_cache.insert(entry.0, entry.1);
                            }
                            tiles.push(b);
                        }
                        None => {
                            tile.state = TileState::Empty;
                        }
                    }
                }
            }

            timing_stop!(ts6);
            (tiles, new_tile_cache)
        }};
    }

    // Stage 6: rasterize tiles using the active backend's rasterizer + strategy (chosen at
    // runtime by the engine's RenderBackend; no per-backend cfg here).
    let (baked_tiles, new_tile_cache) = match (strategy, rasterizer) {
        (RasterStrategy::ParallelCached, Some(rasterizer)) => rasterize_parallel!(rasterizer),
        (RasterStrategy::Sequential, Some(rasterizer)) => {
            rasterize_sequential(rasterizer, &layer_ids, &mut tile_list, full_page_rect, &media_store)
        }
        _ => (Vec::new(), std::collections::HashMap::new()),
    };

    timing_stop!(ts_total);

    // Pre-build the CachedTile list for zero-copy scroll handles.
    let cached_tiles = Arc::new(
        baked_tiles
            .iter()
            .map(|t| CachedTile {
                page_x: t.page_x as f32,
                page_y: t.page_y as f32,
                width: t.width,
                height: t.height,
                data: t.data.clone(),
                format: t.format,
            })
            .collect::<Vec<_>>(),
    );

    PipelineCache {
        tiles: baked_tiles,
        page_height,
        cached_tiles,
        layer_list: saved_layer_list,
        tile_pixel_cache: new_tile_cache,
    }
}

/// Hover-only repaint: skip stages 1–2 (render-tree + layout), reuse the cached
/// `LayerList`, and only repaint tiles that intersect the old or new hovered element.
/// All other tiles are carried over from `prev_baked_tiles` unchanged — no CSS
/// re-evaluation, no re-rasterization.
#[allow(clippy::too_many_arguments)]
fn pipeline_hover_repaint(
    layer_list: Arc<gosub_render_pipeline::layering::layer::LayerList>,
    page_height: f64,
    prev_baked_tiles: Vec<BakedTile>,
    old_hover_lei: Option<LayoutElementId>,
    new_hover_lei: Option<LayoutElementId>,
    hover_dirty_nodes: &[NodeId],
    viewport: &gosub_render_pipeline::render::Viewport,
    rasterizer: Option<&(dyn Rasterable + Send + Sync)>,
    strategy: RasterStrategy,
    prev_tile_cache: TilePixelCache,
    media_store: Arc<gosub_render_pipeline::common::media::MediaStore>,
) -> PipelineCache {
    use gosub_render_pipeline::common::browser_state::{BrowserState, WireframeState};
    use gosub_render_pipeline::common::geo::{Dimension as PipelineDimension, Rect as PipelineRect};
    use gosub_render_pipeline::painter::Painter;
    use gosub_render_pipeline::tiler::{TileList, TileState};
    use gosub_shared::{timing_start, timing_stop};

    // Stage 4: tiling — reuse existing LayerList, no layout work.
    let ts4 = timing_start!("pipeline.hover.tiling");
    let mut tile_list = TileList::from_arc(Arc::clone(&layer_list), PipelineDimension::new(256.0, 256.0));
    tile_list.generate();
    let total_tiles = tile_list.arena.len();
    timing_stop!(ts4);

    // Build a position-keyed lookup of previous baked tiles so non-hover tiles can be
    // carried over without any CSS re-evaluation or rasterization.
    // Key: (page_x bits, page_y bits) — deterministic since tile positions don't change.
    let mut prev_by_pos: std::collections::HashMap<(u64, u64), BakedTile> = prev_baked_tiles
        .into_iter()
        .map(|t| ((t.page_x.to_bits(), t.page_y.to_bits()), t))
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
            if !overlaps {
                tile.state = TileState::Clean;
                let key = (tile_rect.x.to_bits(), tile_rect.y.to_bits());
                if let Some(baked) = prev_by_pos.remove(&key) {
                    clean_baked.push(baked);
                }
            } else {
                // Invalidate cached styles only for the hover-chain nodes (old + new ancestors).
                // Non-hover elements in this tile keep their cached CSS — only the nodes that
                // actually gained or lost :hover need re-evaluation.
                doc.invalidate_style_for_nodes(hover_dirty_nodes);
            }
        }
    } else {
        // No hover element visible — reuse all previous baked tiles unchanged.
        let all_tiles: Vec<BakedTile> = prev_by_pos.into_values().collect();
        let cached_tiles = Arc::new(
            all_tiles
                .iter()
                .map(|t| CachedTile {
                    page_x: t.page_x as f32,
                    page_y: t.page_y as f32,
                    width: t.width,
                    height: t.height,
                    data: t.data.clone(),
                    format: t.format,
                })
                .collect::<Vec<_>>(),
        );
        return PipelineCache {
            tiles: all_tiles,
            page_height,
            cached_tiles,
            layer_list,
            tile_pixel_cache: prev_tile_cache,
        };
    }

    // Stage 5: paint ONLY dirty (hover-affected) tiles.
    let render_height = page_height;
    let ts5 = timing_start!("pipeline.hover.painting");
    let full_page_rect = PipelineRect::new(0.0, 0.0, viewport.width as f64, render_height.max(1.0));
    let layer_ids = tile_list.layer_list.layer_ids.read().clone();
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
    let painter = Painter::new(tile_list.layer_list.clone());
    for &layer_id in &layer_ids {
        let tile_ids = tile_list.get_intersecting_tiles(layer_id, full_page_rect);
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
    timing_stop!(ts5);

    // Stage 6: rasterize (parallel for Cairo/Skia, using the tile-pixel cache).
    macro_rules! rasterize_parallel {
        ($rasterizer:expr) => {{
            use gosub_render_pipeline::common::texture_store::TextureStore;
            use gosub_render_pipeline::render::backend::PixelFormat;
            use gosub_render_pipeline::tiler::TileId;
            use rayon::prelude::*;

            // Cairo and Skia both emit premultiplied ARGB32 (BGRA byte order).
            let tile_format = PixelFormat::PreMulArgb32;

            let ts6 = timing_start!("pipeline.hover.rasterize");
            // `media_store` is the shared store passed into pipeline_hover_repaint. It still
            // holds media loaded by the last full layout. &Arc derefs to &MediaStore.
            let rasterizer = $rasterizer;

            let dirty_ids: Vec<TileId> = layer_ids
                .iter()
                .flat_map(|&layer_id| tile_list.get_intersecting_tiles(layer_id, full_page_rect))
                .filter(|&id| tile_list.arena.get(&id).map_or(false, |t| t.state == TileState::Dirty))
                .collect();

            type CacheEntry = (TileCacheKey, (u32, u32, bytes::Bytes));
            let results: Vec<(TileId, Option<BakedTile>, Option<CacheEntry>)> = dirty_ids
                .par_iter()
                .map(|&tile_id| {
                    let Some(tile) = tile_list.arena.get(&tile_id) else {
                        return (tile_id, None, None);
                    };
                    let key = tile_cache_key(tile);
                    if let Some(&(w, h, ref data)) = prev_tile_cache.get(&key) {
                        return (
                            tile_id,
                            Some(BakedTile {
                                page_x: tile.rect.x,
                                page_y: tile.rect.y,
                                width: w,
                                height: h,
                                data: data.clone(),
                                format: tile_format,
                            }),
                            None,
                        );
                    }
                    let mut local_store = TextureStore::new();
                    let baked = rasterizer
                        .rasterize(tile, &mut local_store, &media_store)
                        .and_then(|tid| local_store.get(tid))
                        .map(|tex| BakedTile {
                            page_x: tile.rect.x,
                            page_y: tile.rect.y,
                            width: tex.width as u32,
                            height: tex.height as u32,
                            data: tex.data.clone(),
                            format: tex.format,
                        });
                    let cache_entry = baked.as_ref().map(|b| (key, (b.width, b.height, b.data.clone())));
                    (tile_id, baked, cache_entry)
                })
                .collect();

            let mut tiles: Vec<BakedTile> = Vec::with_capacity(results.len());
            let mut new_tile_cache: TilePixelCache = std::collections::HashMap::with_capacity(results.len());
            for (tile_id, baked, cache_entry) in results {
                if let Some(tile) = tile_list.arena.get_mut(&tile_id) {
                    match baked {
                        Some(b) => {
                            tile.state = TileState::Clean;
                            if let Some(e) = cache_entry {
                                new_tile_cache.insert(e.0, e.1);
                            }
                            tiles.push(b);
                        }
                        None => {
                            tile.state = TileState::Empty;
                        }
                    }
                }
            }
            timing_stop!(ts6);
            (tiles, new_tile_cache)
        }};
    }

    // Stage 6 (hover): rasterize the dirty tiles with the active backend's rasterizer + strategy.
    let (baked_tiles, new_tile_cache) = match (strategy, rasterizer) {
        (RasterStrategy::ParallelCached, Some(rasterizer)) => rasterize_parallel!(rasterizer),
        (RasterStrategy::Sequential, Some(rasterizer)) => {
            rasterize_sequential(rasterizer, &layer_ids, &mut tile_list, full_page_rect, &media_store)
        }
        _ => {
            let _ = &media_store;
            (Vec::new(), std::collections::HashMap::new())
        }
    };

    // Merge: newly rasterized hover tiles + clean tiles carried from previous render.
    let all_baked_tiles: Vec<BakedTile> = baked_tiles.into_iter().chain(clean_baked).collect();

    let cached_tiles = Arc::new(
        all_baked_tiles
            .iter()
            .map(|t| gosub_render_pipeline::render::backend::CachedTile {
                page_x: t.page_x as f32,
                page_y: t.page_y as f32,
                width: t.width,
                height: t.height,
                data: t.data.clone(),
                format: t.format,
            })
            .collect::<Vec<_>>(),
    );

    PipelineCache {
        tiles: all_baked_tiles,
        page_height,
        cached_tiles,
        layer_list,
        tile_pixel_cache: new_tile_cache,
    }
}

/// Stage 7: composite visible tiles from the cache into `rl`.
///
/// Selects tiles that intersect `(scroll_x, scroll_y, vp_w, vp_h)` and blits them at
/// screen-relative positions. This is the only work done on every scroll tick.
fn pipeline_composite(cache: &PipelineCache, scroll_x: f64, scroll_y: f64, vp_w: f64, vp_h: f64, rl: &mut RenderList) {
    use gosub_shared::{timing_start, timing_stop};
    let ts7 = timing_start!("pipeline.composite");

    for tile in &cache.tiles {
        // Cull tiles fully outside the viewport.
        if tile.page_x + tile.width as f64 <= scroll_x {
            continue;
        }
        if tile.page_y + tile.height as f64 <= scroll_y {
            continue;
        }
        if tile.page_x >= scroll_x + vp_w {
            continue;
        }
        if tile.page_y >= scroll_y + vp_h {
            continue;
        }

        rl.items.push(DisplayItem::Blit {
            x: (tile.page_x - scroll_x) as f32,
            y: (tile.page_y - scroll_y) as f32,
            w: tile.width,
            h: tile.height,
            data: tile.data.clone(),
            format: tile.format,
        });
    }

    timing_stop!(ts7);
}
