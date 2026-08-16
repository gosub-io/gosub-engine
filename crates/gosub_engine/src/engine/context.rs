//! [`BrowsingContext`]: the runtime state for a single tab's document and rendering -
//! the parsed DOM, viewport, dirty-flag tracking, storage handles, and the pipeline
//! caches (tiles, render list, GPU scene) built from them.
//!
//! Loading itself lives in the tab worker; the worker hands a parsed document to the
//! context via `set_document`, after which the context rebuilds whichever render
//! representation the active backend consumes.

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
use gosub_render_pipeline::common::texture::TilePixels;
use gosub_render_pipeline::layering::layer::LayerList;
use gosub_render_pipeline::layouter::LayoutElementId;
use gosub_render_pipeline::painter::{PaintScene, Painter};
use gosub_render_pipeline::render::backend::{CachedTile, ExternalHandle};
use gosub_shared::node::NodeId;
use std::any::Any;

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
/// Cached output of stages 1–6 for the whole page. Re-used on every scroll tick.
struct PipelineCache {
    tiles: Vec<BakedTile>,
    page_height: f64,
    /// Pre-built CachedTile list (Arc-shared pixel data) for zero-copy scroll handles.
    cached_tiles: Arc<Vec<CachedTile>>,
    /// Layer list retained for hit-testing (hover).
    /// `None` for a page rendered out-of-process: the layer list is a
    /// process-local structure. Such a page carries `hit_regions` instead,
    /// which answers hit testing; only hover *repaint* still needs the layer
    /// list (it re-paints tiles), so that stays local-only.
    layer_list: Option<Arc<LayerList>>,
    /// Hit-test geometry for a remotely rendered page, in hit-test order.
    /// Empty for local renders, which hit-test through `layer_list`.
    hit_regions: Vec<crate::fork_server::protocol::HitRegion>,
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

    /// LRU bookkeeping + eviction for the tile caches, bounded by the
    /// `renderer.tile.cache_budget_mb` setting.
    tile_budget: TileBudget,
    /// The loader subresources go through - kept beside the media store (which
    /// also holds it) because an out-of-process render needs it directly: the
    /// broker answers the remote renderer's resource requests with it.
    #[cfg_attr(not(all(feature = "process-isolation", target_os = "linux")), allow(dead_code))]
    loader: std::sync::Arc<dyn gosub_interface::resource_loader::ResourceLoader>,
    /// The source text of the current document, kept when a renderer process
    /// will re-parse it there. `None` when rendering in-process.
    document_source: Option<std::sync::Arc<str>>,
    /// Tiles from the last remote render, keyed by content hash; offered to the
    /// next render so unchanged tiles are neither rasterized nor shipped again.
    /// Remote counterpart of `tile_pixel_cache`.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    remote_tile_memory: crate::fork_server::client::TileMemory,
    /// How this tab renders out-of-process, installed by the tab worker when
    /// `security.renderer_process` is on: through the engine's fork server
    /// (`Full`-tier font systems) or via a fresh exec'd renderer per render
    /// (`FontPathsReadable`).
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    remote_renderer: Option<RemoteRenderer>,
}

/// The two ways a tab's renders leave this process - which one applies is the
/// configured font system's confinement tier, decided statically.
#[cfg(all(feature = "process-isolation", target_os = "linux"))]
pub enum RemoteRenderer {
    /// Fork from the engine's warmed fork server (tier `Full`).
    ForkServer(std::sync::Arc<parking_lot::Mutex<crate::fork_server::client::ForkServer>>),
    /// Spawn a throwaway exec'd renderer per render (tier `FontPathsReadable`:
    /// warming buys nothing when font files stay reachable, and the stack may
    /// not even be constructible in a fork server).
    ExecPerRender,
}

impl<C: RenderConfiguration> BrowsingContext<C> {
    /// Creates a new runtime browsing context, sharing the given per-engine settings store.
    pub(crate) fn new(
        config_store: Config,
        loader: std::sync::Arc<dyn gosub_interface::resource_loader::ResourceLoader>,
    ) -> BrowsingContext<C> {
        // Raster decoding is the single most dangerous thing done with untrusted
        // bytes, so where the setting allows it happens in a throwaway process.
        // Read here rather than passed in: it is a property of how the engine was
        // configured, not of this tab.
        let decoder = image_decoder_from(&config_store);
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
            media_store: std::sync::Arc::new(
                gosub_render_pipeline::common::media::MediaStore::with_loader_and_decoder(loader.clone(), decoder),
            ),
            config_store,
            tile_budget: TileBudget::new(),
            loader,
            document_source: None,
            #[cfg(all(feature = "process-isolation", target_os = "linux"))]
            remote_tile_memory: Default::default(),
            #[cfg(all(feature = "process-isolation", target_os = "linux"))]
            remote_renderer: None,
        }
    }

    /// Route this tab's full renders out-of-process. Installed once by the
    /// tab worker; see [`Self::remote_render_active`] for when it engages.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    pub fn set_remote_renderer(&mut self, renderer: RemoteRenderer) {
        self.remote_renderer = Some(renderer);
    }

    /// Whether full renders go out-of-process: a remote renderer is installed
    /// *and* the current document's source is available to send it.
    #[allow(clippy::needless_return)] // the cfg arms need explicit returns
    pub fn remote_render_active(&self) -> bool {
        #[cfg(all(feature = "process-isolation", target_os = "linux"))]
        {
            return self.remote_renderer.is_some() && self.document_source.is_some();
        }
        #[cfg(not(all(feature = "process-isolation", target_os = "linux")))]
        {
            return false;
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

    /// `source` is the text the document was parsed from, kept when an
    /// out-of-process renderer will need to re-parse it.
    pub fn set_document(&mut self, doc: Arc<EngineDocument<C>>, source: Option<std::sync::Arc<str>>) {
        self.document = Some(doc);
        self.document_source = source;
        self.dom_dirty = true;
        self.style_dirty = true;
        self.layout_dirty = true;
        self.invalidate_render();
        self.pipeline_cache = None;
        self.scene_cache = None;
        self.tile_budget.reset();
        self.hover_dirty = false;
        self.hover_leaf = None;
        self.hover_layout_element = None;
        self.hover_fingerprints = None;
        self.hover_chain_sensitive = false;
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
        // Scrolling toward a region whose tiles were evicted by the cache budget: compositing
        // alone cannot restore pixels, so schedule a full re-render to re-rasterize them.
        if self.tile_budget.needs_rerender(y, self.viewport.height as f64) {
            self.render_dirty = true;
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
        #[cfg(all(feature = "process-isolation", target_os = "linux"))]
        if self.remote_render_active() && self.try_remote_pipeline() {
            // Every tile is live again; an earlier in-process render may have evicted some.
            // Remote pages are otherwise unbudgeted: their pixels are shared with the
            // tile memory that makes re-renders incremental, so evicting here frees nothing.
            self.tile_budget.note_full_raster();
            self.render_dirty = false;
            self.hover_dirty = false;
            self.dom_dirty = false;
            self.style_dirty = false;
            self.layout_dirty = false;
            return;
        }
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
                self.config_store.get_uint("renderer.tile.size") as f64,
            ));
        }
        self.enforce_tile_budget(true);
        self.render_dirty = false;
        self.hover_dirty = false;
        self.dom_dirty = false;
        self.style_dirty = false;
        self.layout_dirty = false;
    }

    /// Apply the `renderer.tile.cache_budget_mb` budget to the current pipeline cache, evicting
    /// LRU tiles outside the near-viewport band. `full_raster` marks that every tile was just
    /// re-rasterized, so previously evicted regions are live again.
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

    /// Render the current document in a forked renderer process and adopt the
    /// result as this tab's pipeline cache.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    fn try_remote_pipeline(&mut self) -> bool {
        use gosub_render_pipeline::common::texture::TilePixels;
        use gosub_render_pipeline::rasterizer::BakedTile;

        let (Some(remote), Some(source)) = (&self.remote_renderer, &self.document_source) else {
            return false;
        };

        // The document's own URL is the renderer's base for relative
        // subresource URLs; about:blank when it has none.
        let page_url = {
            use gosub_interface::document::Document as _;
            self.document
                .as_ref()
                .and_then(|doc| doc.url())
                .map(|url| url.to_string())
                .unwrap_or_else(|| "about:blank".to_string())
        };
        let viewport = (self.viewport.width as f64, self.viewport.height as f64);
        let result = match remote {
            RemoteRenderer::ForkServer(server) => server.lock().render_page(
                source,
                &page_url,
                viewport,
                self.loader.as_ref(),
                &self.remote_tile_memory,
                self.hover_leaf.map(|id| id.into()),
            ),
            RemoteRenderer::ExecPerRender => crate::render_process::client::render_page(
                source,
                &page_url,
                viewport,
                self.loader.as_ref(),
                &self.remote_tile_memory,
                self.hover_leaf.map(|id| id.into()),
            ),
        };
        match result {
            Ok(page) => {
                use crate::fork_server::client::{KeptTile, PageTile};

                // Fresh tiles bring their pixels (mapped, zero-copy); reused
                // ones were never re-rendered, so their pixels - and the
                // physical dimensions the renderer did not produce - come
                // from what this tab kept.
                let mut memory: Vec<(u64, KeptTile)> = Vec::with_capacity(page.tiles.len());
                let baked: Vec<BakedTile> = page
                    .tiles
                    .into_iter()
                    .map(|tile| {
                        let (header, kept) = match tile {
                            PageTile::Fresh { header, mapping } => {
                                let kept = KeptTile {
                                    width: header.width,
                                    height: header.height,
                                    format: header.format,
                                    pixels: bytes::Bytes::from_owner(mapping),
                                };
                                (header, kept)
                            }
                            PageTile::Reused { header, kept } => (header, kept),
                        };
                        memory.push((header.content_hash, kept.clone()));
                        BakedTile {
                            page_x: header.page_x,
                            page_y: header.page_y,
                            layer_id: header.layer_id,
                            width: kept.width,
                            height: kept.height,
                            format: kept.format.into(),
                            opacity: header.opacity,
                            anchor: header.anchor.into(),
                            pixels: TilePixels::Cpu(kept.pixels),
                        }
                    })
                    .collect();
                self.remote_tile_memory.replace_with(memory);
                let cached_tiles = Arc::new(gosub_render_pipeline::rasterizer::cpu_cached_tiles(&baked));
                self.pipeline_cache = Some(PipelineCache {
                    tiles: baked,
                    page_height: page.summary.page_height,
                    cached_tiles,
                    layer_list: None,
                    hit_regions: page.hit_regions,
                    tile_pixel_cache: Default::default(),
                });
                true
            }
            Err(e) => {
                log::warn!("out-of-process render failed ({e}); rendering this page in-process");
                false
            }
        }
    }

    /// Rebuild stages 1-6 (pipeline cache) if content has changed, without building a display
    /// list. Used by TileCache backends (Cairo, Skia, Vello) which composite tiles directly
    /// on the host thread and never consume the render list.
    pub fn rebuild_pipeline_cache_if_needed(&mut self) {
        if !self.render_dirty && !self.hover_dirty && !self.scroll_dirty {
            return;
        }
        if self.render_dirty {
            self.rebuild_full_pipeline();
        } else if self.hover_dirty {
            // Paint-only repaint: reuse the cached layout tree, skip stages 1–2.
            // A remotely rendered page has no layer list to repaint from, so
            // hover effects are a no-op there (see `PipelineCache::layer_list`).
            let has_layer_list = self
                .pipeline_cache
                .as_ref()
                .is_some_and(|cache| cache.layer_list.is_some());
            if !has_layer_list && self.pipeline_cache.is_some() {
                self.hover_dirty = false;
                self.scroll_dirty = false;
                return;
            }
            if let Some(old_cache) = self.pipeline_cache.take() {
                let PipelineCache {
                    layer_list: Some(layer_list),
                    page_height,
                    tile_pixel_cache: prev_tile_cache,
                    tiles: prev_baked_tiles,
                    ..
                } = old_cache
                else {
                    // Checked above: the cache has a layer list; this arm
                    // cannot run, and diverging satisfies let-else.
                    return;
                };
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
                        self.rasterizer.as_deref(),
                        self.raster_strategy,
                        std::collections::HashMap::new(),
                        self.media_store.clone(),
                        self.config_store.get_uint("renderer.tile.size") as f64,
                    ));
                    self.enforce_tile_budget(true);
                }
            }
            self.hover_dirty = false;
        }
        self.scroll_dirty = false;
        self.scene_epoch = self.scene_epoch.wrapping_add(1);
    }

    /// Build/refresh the device-agnostic render list if needed. Content changes rerun
    /// stages 1–6; scroll-only changes re-composite cached tiles without layout or
    /// rasterization work.
    pub fn rebuild_render_list_if_needed(&mut self) {
        if !self.render_dirty && !self.scroll_dirty {
            return;
        }

        if self.render_dirty {
            self.rebuild_full_pipeline();
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
            .or_else(|| self.pipeline_cache.as_ref().and_then(|c| c.layer_list.as_ref()))
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

    /// Hit-test at viewport coordinates `(vp_x, vp_y)` and update hover state.
    pub fn update_hover(&mut self, vp_x: f64, vp_y: f64) -> (bool, bool, Option<String>) {
        let _t_total = gosub_shared::timing_guard!("hover.total");

        let (scroll_x, scroll_y) = (self.scroll_x, self.scroll_y);

        // A remotely rendered page carries hit-test geometry instead of a layer
        // list; the layout element id is unavailable there, which costs hover
        // repaint, not hit testing (see `PipelineCache::hit_regions`).
        if let Some(regions) = self.remote_hit_regions() {
            let node = hit_test_regions(regions, vp_x, vp_y, scroll_x, scroll_y);
            return self.apply_hover(node, None);
        }

        let (new_leaf, new_lei) = self.active_layer_list().map_or((None, None), |layer_list| {
            let _t = gosub_shared::timing_guard!("hover.hit_test");
            // find_element_at handles scroll per-layer (fixed layers ignore it).
            let Some(lei) = layer_list.find_element_at(vp_x, vp_y, scroll_x, scroll_y) else {
                return (None, None);
            };
            let dom_node_id = layer_list.layout_tree.get_node_by_id(lei).map(|el| el.dom_node_id);
            (dom_node_id, Some(lei))
        });

        self.apply_hover(new_leaf, new_lei)
    }

    /// Hit-test geometry for a remotely rendered page, when that is how the
    /// current page was produced.
    fn remote_hit_regions(&self) -> Option<&[crate::fork_server::protocol::HitRegion]> {
        let cache = self.pipeline_cache.as_ref()?;
        (cache.layer_list.is_none() && !cache.hit_regions.is_empty()).then_some(cache.hit_regions.as_slice())
    }

    /// Fold a hit-test result into hover state: ancestor walk for the link and
    /// `:hover` sensitivity, CSS invalidation for the nodes whose hover state
    /// changed, and the repaint decision.
    fn apply_hover(
        &mut self,
        new_leaf: Option<NodeId>,
        new_lei: Option<LayoutElementId>,
    ) -> (bool, bool, Option<String>) {
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
            // A remotely rendered page has no local layout tree to repaint from,
            // so hover falls back to a re-render; the renderer skips tiles with
            // unchanged content, so this stays cheap.
            #[cfg(all(feature = "process-isolation", target_os = "linux"))]
            let remote = self.remote_render_active();
            #[cfg(not(all(feature = "process-isolation", target_os = "linux")))]
            let remote = false;

            if remote {
                self.render_dirty = true;
            } else {
                // Hover-only changes are paint-only (color, background, box-shadow).
                // Use the cheap hover-dirty path which skips render-tree + layout.
                self.hover_dirty = true;
            }
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
    media_store: Arc<gosub_render_pipeline::common::media::MediaStore>,
) -> SceneCache {
    use gosub_render_pipeline::common::browser_state::{BrowserState, WireframeState};
    use gosub_render_pipeline::common::document::pipeline_doc::GosubDocumentAdapter;
    use gosub_render_pipeline::common::geo::{Dimension as PipelineDimension, Rect as PipelineRect};
    use gosub_render_pipeline::layouter::taffy::TaffyLayouter;
    use gosub_render_pipeline::layouter::CanLayout;
    use gosub_render_pipeline::rendertree_builder::RenderTree;

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

/// Runs pipeline stages 1–6 for the entire page (all tiles, not just the viewport slice)
/// and returns a `PipelineCache` of rasterized tiles ready for repeated compositing.
///
/// Splitting the full pipeline from compositing lets scroll re-use the cached tiles without
/// re-running layout or rasterization.
fn pipeline_build_cache<C: RenderConfiguration>(
    doc: Arc<EngineDocument<C>>,
    viewport: &Viewport,
    rasterizer: Option<&(dyn Rasterable + Send + Sync)>,
    strategy: RasterStrategy,
    prev_tile_cache: TilePixelCache,
    media_store: Arc<gosub_render_pipeline::common::media::MediaStore>,
    tile_size: f64,
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
    let painter = Painter::new(tile_list.layer_list.clone(), rasterizer.and_then(|r| r.font_system()));
    for &layer_id in &layer_ids {
        let tile_ids = tile_list.get_intersecting_tiles(layer_id, full_page_rect);
        for tile_id in tile_ids {
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
        layer_list: Some(saved_layer_list),
        hit_regions: Vec::new(),
        tile_pixel_cache: new_tile_cache,
    }
}

/// Which node a point lands on, per a remotely rendered page's geometry.
fn hit_test_regions(
    regions: &[crate::fork_server::protocol::HitRegion],
    vp_x: f64,
    vp_y: f64,
    scroll_x: f64,
    scroll_y: f64,
) -> Option<NodeId> {
    use crate::fork_server::protocol::TileWireAnchor;
    use gosub_render_pipeline::render::backend::StickyConstraint;

    for region in regions {
        let (x, y) = match region.anchor {
            TileWireAnchor::Fixed => (vp_x, vp_y),
            TileWireAnchor::Scroll => (vp_x + scroll_x, vp_y + scroll_y),
            TileWireAnchor::Sticky(s) => {
                let (dx, dy) = StickyConstraint {
                    inset_top: s.inset_top,
                    inset_left: s.inset_left,
                    natural_x: s.natural_x,
                    natural_y: s.natural_y,
                    natural_w: s.natural_w,
                    natural_h: s.natural_h,
                    cage_x: s.cage_x,
                    cage_y: s.cage_y,
                    cage_w: s.cage_w,
                    cage_h: s.cage_h,
                }
                .offset(scroll_x, scroll_y);
                (vp_x + scroll_x - dx, vp_y + scroll_y - dy)
            }
        };
        if x >= region.x && x < region.x + region.width && y >= region.y && y < region.y + region.height {
            return Some(NodeId::from(region.node_id));
        }
    }
    None
}

/// Hover-only repaint: skip stages 1–2 (render-tree + layout), reuse the cached
/// `LayerList`, and only repaint tiles that intersect the old or new hovered element.
/// All other tiles are carried over from `prev_baked_tiles` unchanged - no CSS
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
    tile_size: f64,
) -> PipelineCache {
    use gosub_render_pipeline::common::browser_state::{BrowserState, WireframeState};
    use gosub_render_pipeline::common::geo::{Dimension as PipelineDimension, Rect as PipelineRect};
    use gosub_render_pipeline::painter::Painter;
    use gosub_render_pipeline::tiler::{TileList, TileState};
    use gosub_shared::{timing_start, timing_stop};

    // Stage 4: tiling - reuse existing LayerList, no layout work.
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
            layer_list: Some(layer_list),
            hit_regions: Vec::new(),
            tile_pixel_cache: prev_tile_cache,
        };
    }

    // Stage 5: paint ONLY dirty (hover-affected) tiles. `full_page_rect` and `layer_ids` were
    // computed above (shared with the carry-over ordering).
    let ts5 = timing_start!("pipeline.hover.painting");
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
    for &layer_id in &layer_ids {
        let tile_ids = tile_list.get_intersecting_tiles(layer_id, full_page_rect);
        for tile_id in tile_ids {
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
        layer_list: Some(layer_list),
        hit_regions: Vec::new(),
        tile_pixel_cache: new_tile_cache,
    }
}

/// Re-emit baked tiles in strict back-to-front layer order (the same order a full render
/// produces them). The compositor blits tiles in list order with source-over, so overlapping
/// layers (e.g. the base layer and a `position: sticky`/`fixed` header sharing a page position)
/// must stay layer-ordered or a lower tile paints over a higher one. `by_key` maps
/// `(page_x bits, page_y bits, layer_id)` → tile; positions with no baked tile (empty/transparent)
/// are simply skipped.
fn order_baked_tiles_by_layer(
    tile_list: &gosub_render_pipeline::tiler::TileList,
    layer_ids: &[gosub_render_pipeline::layering::layer::LayerId],
    full_page_rect: gosub_render_pipeline::common::geo::Rect,
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
    use gosub_shared::{timing_start, timing_stop};
    let ts7 = timing_start!("pipeline.composite");

    use gosub_render_pipeline::render::backend::anchored_tile_pos;

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

/// The image decoder this engine should use, if any.
#[cfg(feature = "process-isolation")]
fn image_decoder_from(config: &Config) -> Option<std::sync::Arc<dyn gosub_interface::media_decoder::ImageDecoder>> {
    config
        .get_bool("security.image_decoder_process")
        .then(|| std::sync::Arc::new(crate::decoder_process::client::ProcessImageDecoder) as _)
}

#[cfg(not(feature = "process-isolation"))]
fn image_decoder_from(_config: &Config) -> Option<std::sync::Arc<dyn gosub_interface::media_decoder::ImageDecoder>> {
    None
}

#[cfg(test)]
mod tests {
    use super::parse_clear_color;

    mod tile_budget_integration {
        use super::super::*;
        use crate::engine::settings_store;
        use crate::html::DefaultRenderConfig;
        use gosub_config::settings::Setting;
        use gosub_css3::system::Css3System;
        use gosub_render_pipeline::common::media::MediaStore;
        use gosub_render_pipeline::common::texture::TextureId;
        use gosub_render_pipeline::common::texture_store::TextureStore;
        use gosub_render_pipeline::render::backend::PixelFormat;
        use gosub_render_pipeline::tiler::Tile;

        /// Stub backend rasterizer: fills every tile with opaque pixels, so each baked tile
        /// costs exactly width * height * 4 bytes.
        struct SolidRasterizer;

        impl Rasterable for SolidRasterizer {
            fn rasterize(&self, tile: &Tile, store: &mut TextureStore, _media: &MediaStore) -> Option<TextureId> {
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

        #[test]
        fn tall_page_cache_stays_under_budget_and_scroll_restores_evicted_tiles() {
            const BUDGET_MB: usize = 4;
            const BUDGET_BYTES: usize = BUDGET_MB * 1024 * 1024;

            let config = settings_store::default_config();
            assert!(config
                .set("renderer.tile.cache_budget_mb", Setting::UInt(BUDGET_MB))
                .is_ok());

            // The page has no subresources, so no loader is needed - and no fork server
            // is installed, so the render stays in-process (`source` is irrelevant).
            let mut ctx: BrowsingContext<DefaultRenderConfig> = BrowsingContext::new(
                config,
                Arc::new(gosub_interface::resource_loader::NoResourceLoader),
            );
            ctx.set_rasterizer(Box::new(SolidRasterizer), RasterStrategy::ParallelCached);
            ctx.set_viewport(Viewport {
                x: 0,
                y: 0,
                width: 512,
                height: 256,
            });

            // ~10 000 px tall page: ~40 tile rows x 2 columns ~= 20 MiB unbudgeted.
            let html =
                r#"<html><body style="margin:0"><div style="height:10000px;background:#ddd"></div></body></html>"#;
            let mut doc = gosub_html5::html_compile::<DefaultRenderConfig>(html);
            doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());
            ctx.set_document(Arc::new(doc), None);

            ctx.rebuild_pipeline_cache_if_needed();
            {
                let Some(cache) = ctx.pipeline_cache.as_ref() else {
                    unreachable!("pipeline cache must exist after rebuild");
                };
                assert!(cache.page_height >= 10_000.0, "page must lay out tall");
                assert!(
                    resident_bytes(cache) <= BUDGET_BYTES,
                    "cache must fit the budget: {} > {}",
                    resident_bytes(cache),
                    BUDGET_BYTES
                );
                assert!(has_tile_near(cache, 0.0, 0.5), "viewport tiles must survive");
                assert!(
                    !has_tile_near(cache, 9984.0, 256.0),
                    "bottom of the page must be evicted"
                );
                assert_eq!(
                    cache.cached_tiles.len(),
                    cache.tiles.len(),
                    "compositor list must not keep evicted pixels alive"
                );
            }

            // Scrolling toward an evicted region must schedule a full re-render...
            ctx.set_scroll(0.0, 5000.0);
            assert!(ctx.render_dirty, "scroll into an evicted region must set render_dirty");

            // ...which restores tiles around the new viewport while staying under budget.
            ctx.rebuild_pipeline_cache_if_needed();
            let Some(cache) = ctx.pipeline_cache.as_ref() else {
                unreachable!("pipeline cache must exist after rebuild");
            };
            assert!(
                has_tile_near(cache, 5000.0, 256.0),
                "tiles near the new viewport must be back"
            );
            assert!(resident_bytes(cache) <= BUDGET_BYTES);
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
