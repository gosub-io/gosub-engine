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
use gosub_render_pipeline::render::{Color, DisplayItem, RenderContext, RenderList, Viewport};
use std::sync::Arc;
use url::Url;

#[cfg(feature = "pipeline")]
use crate::html::HtmlEngineConfig;
#[cfg(feature = "pipeline")]
use gosub_interface::document::Document as _;
#[cfg(feature = "pipeline")]
use gosub_render_pipeline::layering::layer::LayerList;
#[cfg(feature = "pipeline")]
use gosub_render_pipeline::render::backend::{CachedTile, ExternalHandle};
#[cfg(feature = "pipeline")]
use gosub_shared::node::NodeId;
// #[derive(Debug, thiserror::Error)]
// pub enum LoadError {
//     #[error("navigation cancelled")]
//     Cancelled,
//     #[error(transparent)]
//     Net(#[from] reqwest::Error),
// }

/// A single rasterized tile with its page-coordinate position, ready to blit.
#[cfg(feature = "pipeline")]
struct BakedTile {
    page_x: f64,
    page_y: f64,
    width: u32,
    height: u32,
    data: Arc<Vec<u8>>,
}

/// Cached output of stages 1–6 for the whole page. Re-used on every scroll tick.
#[cfg(feature = "pipeline")]
struct PipelineCache {
    tiles: Vec<BakedTile>,
    page_height: f64,
    /// Pre-built CachedTile list (Arc-shared pixel data) for zero-copy scroll handles.
    cached_tiles: Arc<Vec<CachedTile>>,
    /// Layer list retained for hit-testing (hover).
    layer_list: Arc<LayerList>,
}

/// BrowsingContext dedicated to a specific tab
///
/// A BrowsingContext is a single instance of the engine that deals with a specific tab. Each tab
/// has one BrowsingContext. These contexts though can use shared processes or threads, but not
/// from other contexts, only from the main engine.
pub struct BrowsingContext {
    // /// Is there anything that needs to be rendered or redrawn?
    // dirty: DirtyFlags,
    /// Current URL being processed
    current_url: Option<Url>,
    /// Parsed DOM document
    document: Option<Arc<EngineDocument>>,
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
    #[cfg(feature = "pipeline")]
    pipeline_cache: Option<PipelineCache>,
    /// The DOM node currently under the pointer (for :hover matching).
    #[cfg(feature = "pipeline")]
    hover_leaf: Option<NodeId>,
    /// The href of the link currently under the pointer, if any.
    #[cfg(feature = "pipeline")]
    pub hover_link_url: Option<String>,

    /// Shared wgpu resources for the Vello rasterizer (device, queue, renderer).
    /// Set by the tab worker when the engine is initialised with a VelloBackend.
    #[cfg(feature = "backend_vello")]
    pub vello_resources: Option<std::sync::Arc<gosub_render_pipeline::render::backends::vello::WgpuResources>>,
}

impl BrowsingContext {
    /// Creates a new runtime browsing context.
    pub(crate) fn new() -> BrowsingContext {
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
            #[cfg(feature = "pipeline")]
            pipeline_cache: None,
            #[cfg(feature = "pipeline")]
            hover_leaf: None,
            #[cfg(feature = "pipeline")]
            hover_link_url: None,
            #[cfg(feature = "backend_vello")]
            vello_resources: None,
        }
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
    pub fn set_document(&mut self, doc: Arc<EngineDocument>) {
        self.document = Some(doc);
        self.dom_dirty = true;
        self.style_dirty = true;
        self.layout_dirty = true;
        self.invalidate_render();
        #[cfg(feature = "pipeline")]
        {
            self.pipeline_cache = None;
            self.hover_leaf = None;
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
            #[cfg(feature = "pipeline")]
            {
                self.pipeline_cache = None;
            }
        }
    }

    /// Update the scroll offset without triggering a full re-layout.
    /// The next composite will shift tiles by (x, y).
    pub fn set_scroll(&mut self, x: f64, y: f64) {
        let x = x.max(0.0);
        #[cfg(feature = "pipeline")]
        let max_y = self
            .pipeline_cache
            .as_ref()
            .map(|c| (c.page_height - self.viewport.height as f64).max(0.0))
            .unwrap_or(f64::MAX);
        #[cfg(not(feature = "pipeline"))]
        let max_y = f64::MAX;
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

    /// Build/refresh the device-agnostic scene if needed.
    ///
    /// Two paths:
    /// - **Full pipeline** (`render_dirty`): runs stages 1–6 for the whole page, caches tiles,
    ///   then composites. Triggered by navigation, DOM/style changes, or viewport resize.
    ///
    /// Rebuild stages 1-6 (pipeline cache) if content has changed, without building a display
    /// list. Used by TileCache backends (Cairo, Skia, Vello) which composite tiles directly
    /// on the host thread and never consume the render list.
    #[cfg(feature = "pipeline")]
    pub fn rebuild_pipeline_cache_if_needed(&mut self) {
        if !self.render_dirty && !self.scroll_dirty {
            return;
        }
        if self.render_dirty {
            if let Some(doc) = &self.document {
                self.pipeline_cache = Some(pipeline_build_cache(
                    doc.clone(),
                    &self.viewport,
                    #[cfg(feature = "backend_vello")]
                    self.vello_resources.clone(),
                ));
            }
            self.render_dirty = false;
            self.dom_dirty = false;
            self.style_dirty = false;
            self.layout_dirty = false;
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

        #[cfg(feature = "pipeline")]
        {
            if self.render_dirty {
                if let Some(doc) = &self.document {
                    self.pipeline_cache = Some(pipeline_build_cache(
                        doc.clone(),
                        &self.viewport,
                        #[cfg(feature = "backend_vello")]
                        self.vello_resources.clone(),
                    ));
                }
                self.render_dirty = false;
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

        #[cfg(not(feature = "pipeline"))]
        {
            let mut rl = RenderList::default();
            if self.document.is_none() {
                rl.items.push(DisplayItem::Clear {
                    color: Color::new(0.6, 0.6, 0.6, 1.0),
                });
            }
            self.render_list = rl;
            self.render_dirty = false;
        }

        self.scroll_dirty = false;
        self.scene_epoch = self.scene_epoch.wrapping_add(1);
    }

    /// If only the scroll offset changed (no content/layout change), returns a zero-copy
    /// `ExternalHandle::TileCache` that the host can composite directly, bypassing the Cairo
    /// render pipeline entirely. Returns `None` when a full render is needed.
    ///
    /// Calling this consumes the scroll-dirty flag and advances the scene epoch.
    #[cfg(feature = "pipeline")]
    pub fn take_scroll_handle(&mut self, dpr: u32) -> Option<ExternalHandle> {
        if !self.scroll_dirty || self.render_dirty {
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
    #[cfg(feature = "pipeline")]
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

    /// Returns the full page height from the last pipeline cache (0 if not yet rendered).
    pub fn page_height(&self) -> f64 {
        #[cfg(feature = "pipeline")]
        return self.pipeline_cache.as_ref().map(|c| c.page_height).unwrap_or(0.0);
        #[cfg(not(feature = "pipeline"))]
        return 0.0;
    }

    /// Hit-test at viewport coordinates `(vp_x, vp_y)` and update hover state.
    /// Returns `(dirty, link_url)`: dirty=true when the hovered element changed,
    /// link_url=Some(href) when the cursor is over a link (None otherwise).
    #[cfg(feature = "pipeline")]
    pub fn update_hover(&mut self, vp_x: f64, vp_y: f64) -> (bool, Option<String>) {
        let page_x = vp_x + self.scroll_x;
        let page_y = vp_y + self.scroll_y;

        let new_leaf = self.pipeline_cache.as_ref().and_then(|cache| {
            let lei = cache.layer_list.find_element_at(page_x, page_y)?;
            let el = cache.layer_list.layout_tree.get_node_by_id(lei)?;
            Some(el.dom_node_id)
        });

        // Walk the ancestor chain to find the nearest <a href>.
        let link_url = new_leaf.and_then(|leaf| {
            let doc = self.document.as_ref()?;
            let mut id = leaf;
            loop {
                if doc.tag_name(id) == Some("a") {
                    if let Some(href) = doc.attribute(id, "href") {
                        return Some(href.to_string());
                    }
                }
                id = doc.parent(id)?;
            }
        });

        self.hover_link_url = link_url.clone();

        if new_leaf == self.hover_leaf {
            return (false, link_url);
        }

        self.hover_leaf = new_leaf;

        if let Some(doc) = &self.document {
            doc.set_hovered_nodes(new_leaf);
        }

        self.render_dirty = true;
        self.style_dirty = true;
        (true, link_url)
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

impl RenderContext for BrowsingContext {
    fn viewport(&self) -> &Viewport {
        &self.viewport
    }
    fn render_list(&self) -> &RenderList {
        &self.render_list
    }
}

/// Runs pipeline stages 1–6 for the **entire page** (all tiles, not just the viewport slice)
/// and returns a `PipelineCache` of rasterized tiles ready for repeated compositing.
///
/// Splitting the full pipeline from compositing lets scroll re-use the cached tiles without
/// re-running layout or rasterization.
#[cfg(feature = "pipeline")]
fn pipeline_build_cache(
    doc: Arc<EngineDocument>,
    viewport: &Viewport,
    #[cfg(feature = "backend_vello")] _vello_resources: Option<
        std::sync::Arc<gosub_render_pipeline::render::backends::vello::WgpuResources>,
    >,
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
    use std::time::Instant;

    log::info!(
        "[pipeline] build cache (viewport {}x{})",
        viewport.width,
        viewport.height
    );
    let t_total = Instant::now();
    let ts_total = timing_start!("pipeline.total");

    // Stage 1: render tree
    let t = Instant::now();
    let ts1 = timing_start!("pipeline.render_tree");
    let adapter = GosubDocumentAdapter::<HtmlEngineConfig>::new(doc);
    let mut render_tree = RenderTree::new(Arc::new(adapter));
    render_tree.parse();
    timing_stop!(ts1);
    log::info!(
        "[pipeline] stage 1 render-tree:  {:>6.1}ms  ({} nodes)",
        t.elapsed().as_secs_f64() * 1000.0,
        render_tree.arena.len()
    );

    let vp_dim = if viewport.width > 0 && viewport.height > 0 {
        Some(PipelineDimension::new(viewport.width as f64, viewport.height as f64))
    } else {
        None
    };

    // Stage 2: layout
    let t = Instant::now();
    let ts2 = timing_start!("pipeline.layout");
    let mut layouter = TaffyLayouter::new();
    let layout_tree = layouter.layout(render_tree, vp_dim, 1.0);
    timing_stop!(ts2);
    let page_height = layout_tree.root_dimension.height;
    log::info!(
        "[pipeline] stage 2 layout:        {:>6.1}ms  (root {}x{:.0})",
        t.elapsed().as_secs_f64() * 1000.0,
        layout_tree.root_dimension.width,
        page_height
    );

    // Stage 3: layering
    let t = Instant::now();
    let ts3 = timing_start!("pipeline.layering");
    let layer_list = LayerList::new(layout_tree);
    let layer_count = layer_list.layer_ids.read().len();
    timing_stop!(ts3);
    log::info!(
        "[pipeline] stage 3 layering:      {:>6.1}ms  ({} layers)",
        t.elapsed().as_secs_f64() * 1000.0,
        layer_count
    );

    // Stage 4: tiling
    let t = Instant::now();
    let ts4 = timing_start!("pipeline.tiling");
    let mut tile_list = TileList::new(layer_list, PipelineDimension::new(256.0, 256.0));
    let saved_layer_list = Arc::clone(&tile_list.layer_list);
    tile_list.generate();
    let total_tiles = tile_list.arena.len();
    timing_stop!(ts4);
    log::info!(
        "[pipeline] stage 4 tiling:        {:>6.1}ms  ({} tiles total)",
        t.elapsed().as_secs_f64() * 1000.0,
        total_tiles
    );

    // Stage 5: paint ALL tiles (full-page rect so nothing is culled).
    let t = Instant::now();
    let ts5 = timing_start!("pipeline.painting");
    let full_page_rect = PipelineRect::new(0.0, 0.0, viewport.width as f64, page_height.max(viewport.height as f64));
    let layer_ids = tile_list.layer_list.layer_ids.read().clone();
    let paint_state = BrowserState {
        visible_layer_list: vec![true; layer_ids.len()],
        wireframed: WireframeState::None,
        debug_hover: false,
        current_hovered_element: None,
        show_tilegrid: false,
        viewport: full_page_rect,
        tile_list: None,
        dpi_scale_factor: 1.0,
    };
    let painter = Painter::new(tile_list.layer_list.clone());
    let mut painted_tiles = 0usize;
    let mut painted_commands = 0usize;
    for &layer_id in &layer_ids {
        let tile_ids = tile_list.get_intersecting_tiles(layer_id, full_page_rect);
        for tile_id in tile_ids {
            if let Some(tile) = tile_list.get_tile_mut(tile_id) {
                if tile.state == TileState::Dirty {
                    let mut cmd_count = 0usize;
                    for tiled_element in &mut tile.elements {
                        let cmds = painter.paint(tiled_element, &paint_state);
                        cmd_count += cmds.len();
                        tiled_element.paint_commands = cmds;
                    }
                    painted_tiles += 1;
                    painted_commands += cmd_count;
                }
            }
        }
    }
    timing_stop!(ts5);
    log::info!(
        "[pipeline] stage 5 painting:      {:>6.1}ms  ({} tiles painted, {} commands total)",
        t.elapsed().as_secs_f64() * 1000.0,
        painted_tiles,
        painted_commands
    );

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
    #[cfg(any(feature = "backend_cairo", feature = "backend_skia"))]
    macro_rules! rasterize_parallel {
        ($rasterizer:expr, $label:literal) => {{
            use gosub_render_pipeline::common::media::MediaStore;
            use gosub_render_pipeline::common::texture_store::TextureStore;
            use gosub_render_pipeline::rasterizer::Rasterable;
            use rayon::prelude::*;

            let t = Instant::now();
            let ts6 = timing_start!("pipeline.rasterize");
            let media_store = MediaStore::new();
            let rasterizer = $rasterizer;

            // Phase 1: collect IDs of dirty tiles across all layers.
            let dirty_ids: Vec<TileId> = layer_ids
                .iter()
                .flat_map(|&layer_id| tile_list.get_intersecting_tiles(layer_id, full_page_rect))
                .filter(|&id| tile_list.arena.get(&id).map_or(false, |t| t.state == TileState::Dirty))
                .collect();

            // Phase 2: parallel rasterization — no shared mutable state.
            let results: Vec<(TileId, Option<BakedTile>)> = dirty_ids
                .par_iter()
                .map(|&tile_id| {
                    let baked = tile_list.arena.get(&tile_id).and_then(|tile| {
                        let mut local_store = TextureStore::new();
                        let tex_id = rasterizer.rasterize(tile, &mut local_store, &media_store)?;
                        let tex = local_store.get(tex_id)?;
                        Some(BakedTile {
                            page_x: tile.rect.x,
                            page_y: tile.rect.y,
                            width: tex.width as u32,
                            height: tex.height as u32,
                            data: Arc::clone(&tex.data),
                        })
                    });
                    (tile_id, baked)
                })
                .collect();

            // Phase 3: update tile states and gather BakedTiles (sequential).
            let mut rasterized = 0usize;
            let mut empty = 0usize;
            let mut tiles: Vec<BakedTile> = Vec::with_capacity(results.len());
            for (tile_id, baked) in results {
                if let Some(tile) = tile_list.arena.get_mut(&tile_id) {
                    match baked {
                        Some(b) => {
                            tile.state = TileState::Clean;
                            tiles.push(b);
                            rasterized += 1;
                        }
                        None => {
                            tile.state = TileState::Empty;
                            empty += 1;
                        }
                    }
                }
            }

            timing_stop!(ts6);
            log::info!(
                concat!("[pipeline] stage 6 rasterize ", $label, " {:>6.1}ms  ({} clean, {} empty)"),
                t.elapsed().as_secs_f64() * 1000.0,
                rasterized,
                empty
            );
            tiles
        }};
    }

    #[cfg(feature = "backend_cairo")]
    let baked_tiles = {
        use gosub_renderer_cairo::CairoRasterizer;
        rasterize_parallel!(CairoRasterizer::new(), "(cairo):    ")
    };

    #[cfg(all(feature = "backend_skia", not(feature = "backend_cairo")))]
    let baked_tiles = {
        use gosub_renderer_skia::SkiaRasterizer;
        rasterize_parallel!(SkiaRasterizer::new(1.0), "(skia):     ")
    };

    #[cfg(all(
        feature = "backend_vello",
        not(feature = "backend_cairo"),
        not(feature = "backend_skia")
    ))]
    let baked_tiles = {
        use gosub_render_pipeline::common::media::MediaStore;
        use gosub_render_pipeline::common::texture_store::TextureStore;
        use gosub_render_pipeline::rasterizer::Rasterable;
        use gosub_renderer_vello::VelloRasterizer;

        let t = Instant::now();
        let ts6 = timing_start!("pipeline.rasterize");
        let media_store = MediaStore::new();
        let mut texture_store = TextureStore::new();
        let mut rasterized = 0usize;
        let mut empty = 0usize;

        let tiles = if let Some(ref resources) = _vello_resources {
            let rasterizer = VelloRasterizer::new(std::sync::Arc::clone(resources));
            for &layer_id in &layer_ids {
                let tile_ids = tile_list.get_intersecting_tiles(layer_id, full_page_rect);
                for tile_id in tile_ids {
                    if let Some(tile) = tile_list.get_tile_mut(tile_id) {
                        if tile.state == TileState::Dirty {
                            match rasterizer.rasterize(tile, &mut texture_store, &media_store) {
                                Some(texture_id) => {
                                    tile.texture_id = Some(texture_id);
                                    tile.state = TileState::Clean;
                                    rasterized += 1;
                                }
                                None => {
                                    tile.state = TileState::Empty;
                                    empty += 1;
                                }
                            }
                        }
                    }
                }
            }
            let mut tiles: Vec<BakedTile> = Vec::with_capacity(rasterized);
            for tile in tile_list.arena.values() {
                if let (Some(texture_id), true) = (tile.texture_id, tile.state == TileState::Clean) {
                    if let Some(tex) = texture_store.get(texture_id) {
                        tiles.push(BakedTile {
                            page_x: tile.rect.x,
                            page_y: tile.rect.y,
                            width: tex.width as u32,
                            height: tex.height as u32,
                            data: Arc::clone(&tex.data),
                        });
                    }
                }
            }
            tiles
        } else {
            log::warn!("[pipeline] backend_vello active but no wgpu resources set — stage 6 skipped");
            Vec::new()
        };

        timing_stop!(ts6);
        log::info!(
            "[pipeline] stage 6 rasterize (vello): {:>6.1}ms  ({} clean, {} empty)",
            t.elapsed().as_secs_f64() * 1000.0,
            rasterized,
            empty
        );
        tiles
    };

    #[cfg(not(any(feature = "backend_cairo", feature = "backend_skia", feature = "backend_vello")))]
    let baked_tiles: Vec<BakedTile> = Vec::new();

    timing_stop!(ts_total);
    log::info!(
        "[pipeline] total (build cache): {:.1}ms  ({} baked tiles)",
        t_total.elapsed().as_secs_f64() * 1000.0,
        baked_tiles.len()
    );

    // Pre-build the CachedTile list for zero-copy scroll handles.
    let cached_tiles = Arc::new(
        baked_tiles
            .iter()
            .map(|t| CachedTile {
                page_x: t.page_x as f32,
                page_y: t.page_y as f32,
                width: t.width,
                height: t.height,
                data: Arc::clone(&t.data),
            })
            .collect::<Vec<_>>(),
    );

    PipelineCache {
        tiles: baked_tiles,
        page_height,
        cached_tiles,
        layer_list: saved_layer_list,
    }
}

/// Stage 7: composite visible tiles from the cache into `rl`.
///
/// Selects tiles that intersect `(scroll_x, scroll_y, vp_w, vp_h)` and blits them at
/// screen-relative positions. This is the only work done on every scroll tick.
#[cfg(feature = "pipeline")]
fn pipeline_composite(cache: &PipelineCache, scroll_x: f64, scroll_y: f64, vp_w: f64, vp_h: f64, rl: &mut RenderList) {
    use gosub_shared::{timing_start, timing_stop};
    let ts7 = timing_start!("pipeline.composite");
    let mut blits = 0usize;

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
            data: Arc::clone(&tile.data),
        });
        blits += 1;
    }

    timing_stop!(ts7);
    log::debug!("[pipeline] stage 7 composite: {} blit items", blits);
}
