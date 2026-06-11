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
use gosub_css3::stylesheet::CssSelectorPart;
#[cfg(feature = "pipeline")]
use gosub_interface::document::Document as _;
#[cfg(feature = "pipeline")]
use gosub_render_pipeline::layering::layer::LayerList;
#[cfg(feature = "pipeline")]
use gosub_render_pipeline::layouter::LayoutElementId;
#[cfg(feature = "pipeline")]
use gosub_render_pipeline::render::backend::{CachedTile, ExternalHandle};
#[cfg(feature = "pipeline")]
use gosub_shared::node::NodeId;

/// Fingerprints of nodes that are the subject of a `:hover` CSS rule.
/// Built once per document load; used to skip style recalcs when hover moves
/// between elements that have no `:hover` rules.
#[cfg(feature = "pipeline")]
#[derive(Default)]
struct HoverFingerprints {
    /// True when a bare `:hover` or `*:hover` rule exists — every node is sensitive.
    has_universal: bool,
    types: std::collections::HashSet<String>,
    classes: std::collections::HashSet<String>,
    ids: std::collections::HashSet<String>,
}

#[cfg(feature = "pipeline")]
impl HoverFingerprints {
    fn empty() -> Self {
        Self::default()
    }

    /// Scan all stylesheets in `doc` and collect the hover-subject fingerprints.
    fn build(doc: &EngineDocument) -> Self {
        let mut fp = Self::empty();
        let sheet_count = doc.stylesheets().len();

        for sheet in doc.stylesheets() {
            for rule in &sheet.rules {
                for selector in &rule.selectors {
                    for part_list in &selector.parts {
                        // Split the part list into compounds (groups between Combinators).
                        // :hover belongs to the compound it appears in; that compound's
                        // Type/Class/Id parts are the hover-subject fingerprint.
                        let mut compound: Vec<&CssSelectorPart> = Vec::new();
                        for part in part_list {
                            if matches!(part, CssSelectorPart::Combinator(_)) {
                                compound.clear();
                                continue;
                            }
                            compound.push(part);
                            if !matches!(part, CssSelectorPart::PseudoClass(n) if n == "hover") {
                                continue;
                            }
                            // Found :hover — classify this compound.
                            let mut specific = false;
                            for p in &compound {
                                match p {
                                    CssSelectorPart::Type(t) => {
                                        fp.types.insert(t.clone());
                                        specific = true;
                                    }
                                    CssSelectorPart::Class(c) => {
                                        fp.classes.insert(c.clone());
                                        specific = true;
                                    }
                                    CssSelectorPart::Id(id) => {
                                        fp.ids.insert(id.clone());
                                        specific = true;
                                    }
                                    _ => {}
                                }
                            }
                            if !specific {
                                // Bare :hover or *:hover — everything is sensitive.
                                fp.has_universal = true;
                                log::info!(
                                    "[hover] fingerprints: universal :hover (from {} stylesheet(s))",
                                    sheet_count
                                );
                                return fp;
                            }
                        }
                    }
                }
            }
        }

        log::info!(
            "[hover] fingerprints built from {} stylesheet(s): types={:?}  classes={:?}  ids={:?}  universal={}",
            sheet_count,
            fp.types.iter().collect::<Vec<_>>(),
            fp.classes.iter().collect::<Vec<_>>(),
            fp.ids.iter().collect::<Vec<_>>(),
            fp.has_universal,
        );
        fp
    }

    fn matches(&self, doc: &EngineDocument, node_id: NodeId) -> bool {
        if self.has_universal {
            return true;
        }
        if let Some(tag) = doc.tag_name(node_id) {
            if self.types.contains(tag) {
                return true;
            }
        }
        for cls in &self.classes {
            if doc.has_class(node_id, cls) {
                return true;
            }
        }
        if !self.ids.is_empty() {
            if let Some(id_attr) = doc.attribute(node_id, "id") {
                if self.ids.contains(id_attr) {
                    return true;
                }
            }
        }
        false
    }
}
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

/// Key that uniquely identifies a tile's content for cache lookup.
/// Format: (page_x bits, page_y bits, layer_id, paint-command hash).
#[cfg(feature = "pipeline")]
type TileCacheKey = (u64, u64, u64, u64);

/// Cached output of stages 1–6 for the whole page. Re-used on every scroll tick.
#[cfg(feature = "pipeline")]
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
    tile_pixel_cache: std::collections::HashMap<TileCacheKey, (u32, u32, Arc<Vec<u8>>)>,
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
    /// Set when only hover state changed — triggers a paint-only repaint (stages 4–6),
    /// skipping the expensive render-tree rebuild (stage 1) and layout (stage 2).
    #[cfg(feature = "pipeline")]
    hover_dirty: bool,
    /// The DOM node currently under the pointer (for :hover matching).
    #[cfg(feature = "pipeline")]
    hover_leaf: Option<NodeId>,
    /// Layout element ID from the PREVIOUS hover update (needed to find which tile to repaint).
    #[cfg(feature = "pipeline")]
    hover_old_lei: Option<LayoutElementId>,
    /// DOM nodes whose hover state changed in the last update (old chain ∪ new chain).
    /// Only these nodes need their cached CSS invalidated; everything else in the tile stays cached.
    #[cfg(feature = "pipeline")]
    hover_dirty_nodes: Vec<NodeId>,
    /// The layout element currently under the pointer, used for bounding-box pre-check.
    #[cfg(feature = "pipeline")]
    hover_layout_element: Option<LayoutElementId>,
    /// Cached :hover fingerprints for the current document; rebuilt on document change.
    #[cfg(feature = "pipeline")]
    hover_fingerprints: Option<HoverFingerprints>,
    /// True when the last hover chain contained a fingerprint-sensitive node.
    #[cfg(feature = "pipeline")]
    hover_chain_sensitive: bool,
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
            hover_dirty: false,
            #[cfg(feature = "pipeline")]
            hover_leaf: None,
            #[cfg(feature = "pipeline")]
            hover_old_lei: None,
            #[cfg(feature = "pipeline")]
            hover_dirty_nodes: Vec::new(),
            #[cfg(feature = "pipeline")]
            hover_layout_element: None,
            #[cfg(feature = "pipeline")]
            hover_fingerprints: None,
            #[cfg(feature = "pipeline")]
            hover_chain_sensitive: false,
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
                    #[cfg(feature = "backend_vello")]
                    self.vello_resources.clone(),
                    prev_tile_cache,
                ));
            }
            self.render_dirty = false;
            self.hover_dirty = false;
            self.dom_dirty = false;
            self.style_dirty = false;
            self.layout_dirty = false;
        } else if self.hover_dirty {
            log::warn!("[hover] hover_dirty → dispatching paint-only repaint (stages 4–6)");
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
                    #[cfg(feature = "backend_vello")]
                    self.vello_resources.clone(),
                    prev_tile_cache,
                ));
            } else {
                // No cached layout yet — fall back to a full rebuild.
                if let Some(doc) = &self.document {
                    self.pipeline_cache = Some(pipeline_build_cache(
                        doc.clone(),
                        &self.viewport,
                        #[cfg(feature = "backend_vello")]
                        self.vello_resources.clone(),
                        std::collections::HashMap::new(),
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

        #[cfg(feature = "pipeline")]
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
                        #[cfg(feature = "backend_vello")]
                        self.vello_resources.clone(),
                        prev_tile_cache,
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
    ///
    /// Returns `(visual_dirty, url_changed, link_url)`:
    /// - `visual_dirty`: a node with a `:hover` CSS rule entered or left the hover chain → needs repaint.
    /// - `url_changed`: the link URL under the cursor changed → caller should emit a `HoverUrl` event.
    /// - `link_url`: the href of the nearest `<a>` ancestor, if any.
    #[cfg(feature = "pipeline")]
    pub fn update_hover(&mut self, vp_x: f64, vp_y: f64) -> (bool, bool, Option<String>) {
        let _t_total = gosub_shared::timing_guard!("hover.total");

        let page_x = vp_x + self.scroll_x;
        let page_y = vp_y + self.scroll_y;

        let (new_leaf, new_lei) = self.pipeline_cache.as_ref().map_or((None, None), |cache| {
            let _t = gosub_shared::timing_guard!("hover.hit_test");
            let Some(lei) = cache.layer_list.find_element_at(page_x, page_y) else {
                return (None, None);
            };
            let dom_node_id = cache
                .layer_list
                .layout_tree
                .get_node_by_id(lei)
                .map(|el| el.dom_node_id);
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
        if self.hover_fingerprints.is_none() {
            self.hover_fingerprints = Some(
                self.document
                    .as_ref()
                    .map(|doc| HoverFingerprints::build(doc))
                    .unwrap_or_else(HoverFingerprints::empty),
            );
        }

        log::debug!("[hover] leaf → {:?}  lei={:?}", new_leaf, new_lei);

        // Walk the ancestor chain once for both link detection and fingerprint matching.
        // Terminate early once both are found.
        let (link_url, new_sensitive) = {
            let fps = self.hover_fingerprints.as_ref().unwrap();
            let mut link: Option<String> = None;
            let mut sensitive = false;

            if let (Some(leaf), Some(doc)) = (new_leaf, self.document.as_ref()) {
                let _t = gosub_shared::timing_guard!("hover.ancestor_walk");
                let mut id = leaf;
                loop {
                    if !sensitive && fps.matches(doc, id) {
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

        log::debug!(
            "[hover] visual_dirty={visual_dirty}  url_changed={url_changed}  new_sensitive={new_sensitive}  url={link_url:?}"
        );

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

impl RenderContext for BrowsingContext {
    fn viewport(&self) -> &Viewport {
        &self.viewport
    }
    fn render_list(&self) -> &RenderList {
        &self.render_list
    }
}

/// Runs pipeline stages 1–6 for the **entire page** (all tiles, not just the viewport slice)
/// Compute a stable cache key for a tile: (page_x bits, page_y bits, layer_id, content hash).
/// The content hash covers all paint commands so any visual change produces a different key.
#[cfg(any(feature = "backend_cairo", feature = "backend_skia"))]
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
    prev_tile_cache: std::collections::HashMap<TileCacheKey, (u32, u32, Arc<Vec<u8>>)>,
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
    if let Err(e) = render_tree.parse() {
        // The layouter tolerates a tree without a root; the frame degrades to empty.
        log::error!("Failed to build render tree: {e}");
    }
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

    // Stage 5: paint all tiles for the full page so that scrolling reveals pre-rendered
    // content. We use the full page_height rather than capping to viewport.height; the
    // compositor only ships the visible subset to the screen anyway, so no extra pixels
    // are transferred. Memory is bounded by tile count: at 256×256×4B per tile, a 6 000 px
    // page × 1 280 px wide = ~120 tiles × 256 KB each ≈ 30 MB, which is acceptable.
    let render_height = page_height;
    let t = Instant::now();
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
            use gosub_render_pipeline::tiler::TileId;
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

            // Phase 2: parallel rasterization with dirty-tile cache.
            // For each tile: compute a content hash; if it matches the previous render's cached
            // pixels, reuse them (cache hit). Otherwise rasterize on this thread.
            // Result: (tile_id, Option<BakedTile>, Option<new_cache_entry>)
            type CacheEntry = (TileCacheKey, (u32, u32, Arc<Vec<u8>>));
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
                            data: Arc::clone(data),
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
                            data: Arc::clone(&tex.data),
                        });

                    let cache_entry = baked.as_ref().map(|b| (key, (b.width, b.height, Arc::clone(&b.data))));
                    (tile_id, baked, cache_entry)
                })
                .collect();

            // Phase 3: update tile states, gather BakedTiles, and build the new tile cache.
            let mut rasterized = 0usize;
            let mut cache_hits = 0usize;
            let mut empty = 0usize;
            let mut tiles: Vec<BakedTile> = Vec::with_capacity(results.len());
            let mut new_tile_cache: std::collections::HashMap<TileCacheKey, (u32, u32, Arc<Vec<u8>>)> =
                std::collections::HashMap::with_capacity(results.len());

            for (tile_id, baked, cache_entry) in results {
                if let Some(tile) = tile_list.arena.get_mut(&tile_id) {
                    match baked {
                        Some(b) => {
                            tile.state = TileState::Clean;
                            if let Some(entry) = cache_entry {
                                new_tile_cache.insert(entry.0, entry.1);
                                rasterized += 1;
                            } else {
                                cache_hits += 1;
                            }
                            tiles.push(b);
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
                concat!(
                    "[pipeline] stage 6 rasterize ",
                    $label,
                    " {:>6.1}ms  ({} rasterized, {} hits, {} empty)"
                ),
                t.elapsed().as_secs_f64() * 1000.0,
                rasterized,
                cache_hits,
                empty
            );
            (tiles, new_tile_cache)
        }};
    }

    #[cfg(feature = "backend_cairo")]
    let (baked_tiles, new_tile_cache) = {
        use gosub_renderer_cairo::CairoRasterizer;
        rasterize_parallel!(CairoRasterizer::new(), "(cairo):    ")
    };

    #[cfg(all(feature = "backend_skia", not(feature = "backend_cairo")))]
    let (baked_tiles, new_tile_cache) = {
        use gosub_renderer_skia::SkiaRasterizer;
        rasterize_parallel!(SkiaRasterizer::new(1.0), "(skia):     ")
    };

    #[cfg(all(
        feature = "backend_vello",
        not(feature = "backend_cairo"),
        not(feature = "backend_skia")
    ))]
    let baked_tiles = {
        // prev_tile_cache is used by the cairo/skia parallel rasterizer; vello doesn't
        // implement the dirty-tile cache yet, so acknowledge the parameter here.
        let _ = &prev_tile_cache;
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
    // Vello uses sequential rasterization; dirty-tile cache not yet implemented.
    #[cfg(all(
        feature = "backend_vello",
        not(feature = "backend_cairo"),
        not(feature = "backend_skia")
    ))]
    let new_tile_cache: std::collections::HashMap<TileCacheKey, (u32, u32, Arc<Vec<u8>>)> =
        std::collections::HashMap::new();

    #[cfg(not(any(feature = "backend_cairo", feature = "backend_skia", feature = "backend_vello")))]
    let baked_tiles: Vec<BakedTile> = Vec::new();
    #[cfg(not(any(feature = "backend_cairo", feature = "backend_skia", feature = "backend_vello")))]
    let new_tile_cache: std::collections::HashMap<TileCacheKey, (u32, u32, Arc<Vec<u8>>)> =
        std::collections::HashMap::new();

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
        tile_pixel_cache: new_tile_cache,
    }
}

/// Hover-only repaint: skip stages 1–2 (render-tree + layout), reuse the cached
/// `LayerList`, and only repaint tiles that intersect the old or new hovered element.
/// All other tiles are carried over from `prev_baked_tiles` unchanged — no CSS
/// re-evaluation, no re-rasterization.
#[cfg(feature = "pipeline")]
#[allow(clippy::too_many_arguments)]
fn pipeline_hover_repaint(
    layer_list: Arc<gosub_render_pipeline::layering::layer::LayerList>,
    page_height: f64,
    prev_baked_tiles: Vec<BakedTile>,
    old_hover_lei: Option<LayoutElementId>,
    new_hover_lei: Option<LayoutElementId>,
    hover_dirty_nodes: &[NodeId],
    viewport: &gosub_render_pipeline::render::Viewport,
    #[cfg(feature = "backend_vello")] _vello_resources: Option<
        std::sync::Arc<gosub_render_pipeline::render::backends::vello::WgpuResources>,
    >,
    prev_tile_cache: std::collections::HashMap<TileCacheKey, (u32, u32, Arc<Vec<u8>>)>,
) -> PipelineCache {
    use gosub_render_pipeline::common::browser_state::{BrowserState, WireframeState};
    use gosub_render_pipeline::common::geo::{Dimension as PipelineDimension, Rect as PipelineRect};
    use gosub_render_pipeline::painter::Painter;
    use gosub_render_pipeline::tiler::{TileList, TileState};
    use gosub_shared::{timing_start, timing_stop};
    use std::time::Instant;

    log::info!("[pipeline] hover repaint (skipping stages 1–2)");
    let t_total = Instant::now();

    // Stage 4: tiling — reuse existing LayerList, no layout work.
    let t = Instant::now();
    let ts4 = timing_start!("pipeline.hover.tiling");
    let mut tile_list = TileList::from_arc(Arc::clone(&layer_list), PipelineDimension::new(256.0, 256.0));
    tile_list.generate();
    let total_tiles = tile_list.arena.len();
    timing_stop!(ts4);
    log::warn!(
        "[pipeline] hover stage 4 tiling: {:>6.1}ms  ({} tiles)",
        t.elapsed().as_secs_f64() * 1000.0,
        total_tiles
    );

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
                    data: Arc::clone(&t.data),
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
    let t = Instant::now();
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
    let mut painted_tiles = 0usize;
    let mut total_elements_painted = 0usize;
    for &layer_id in &layer_ids {
        let tile_ids = tile_list.get_intersecting_tiles(layer_id, full_page_rect);
        for tile_id in tile_ids {
            if let Some(tile) = tile_list.get_tile_mut(tile_id) {
                if tile.state == TileState::Dirty {
                    let elem_count = tile.elements.len();
                    let t_tile = Instant::now();
                    let mut cmds = 0usize;
                    for tiled_element in &mut tile.elements {
                        let c = painter.paint(tiled_element, &paint_state);
                        cmds += c.len();
                        tiled_element.paint_commands = c;
                    }
                    log::info!(
                        "[pipeline] hover s5 tile ({:.0},{:.0}) elems={} cmds={} in {:.1}ms",
                        tile.rect.x,
                        tile.rect.y,
                        elem_count,
                        cmds,
                        t_tile.elapsed().as_secs_f64() * 1000.0
                    );
                    painted_tiles += 1;
                    total_elements_painted += elem_count;
                    let _ = cmds;
                }
            }
        }
    }
    timing_stop!(ts5);
    log::warn!(
        "[pipeline] hover stage 5 painting: {:>6.1}ms  ({} dirty tiles / {} elems painted, {} clean reused)",
        t.elapsed().as_secs_f64() * 1000.0,
        painted_tiles,
        total_elements_painted,
        clean_baked.len()
    );

    // Stage 6: rasterize (parallel for Cairo/Skia, using the tile-pixel cache).
    #[cfg(any(feature = "backend_cairo", feature = "backend_skia"))]
    macro_rules! rasterize_parallel {
        ($rasterizer:expr, $label:literal) => {{
            use gosub_render_pipeline::common::media::MediaStore;
            use gosub_render_pipeline::common::texture_store::TextureStore;
            use gosub_render_pipeline::rasterizer::Rasterable;
            use gosub_render_pipeline::tiler::TileId;
            use rayon::prelude::*;

            let t = Instant::now();
            let ts6 = timing_start!("pipeline.hover.rasterize");
            let media_store = MediaStore::new();
            let rasterizer = $rasterizer;

            let t_dirty = Instant::now();
            let dirty_ids: Vec<TileId> = layer_ids
                .iter()
                .flat_map(|&layer_id| tile_list.get_intersecting_tiles(layer_id, full_page_rect))
                .filter(|&id| tile_list.arena.get(&id).map_or(false, |t| t.state == TileState::Dirty))
                .collect();
            log::info!(
                "[pipeline] hover s6 dirty_ids: {:.1}ms ({} dirty, {} layers)",
                t_dirty.elapsed().as_secs_f64() * 1000.0,
                dirty_ids.len(),
                layer_ids.len()
            );

            type CacheEntry = (TileCacheKey, (u32, u32, Arc<Vec<u8>>));
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
                                data: Arc::clone(data),
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
                            data: Arc::clone(&tex.data),
                        });
                    let cache_entry = baked.as_ref().map(|b| (key, (b.width, b.height, Arc::clone(&b.data))));
                    (tile_id, baked, cache_entry)
                })
                .collect();

            let mut rasterized = 0usize;
            let mut cache_hits = 0usize;
            let mut tiles: Vec<BakedTile> = Vec::with_capacity(results.len());
            let mut new_tile_cache: std::collections::HashMap<TileCacheKey, (u32, u32, Arc<Vec<u8>>)> =
                std::collections::HashMap::with_capacity(results.len());
            for (tile_id, baked, cache_entry) in results {
                if let Some(tile) = tile_list.arena.get_mut(&tile_id) {
                    match baked {
                        Some(b) => {
                            tile.state = TileState::Clean;
                            if let Some(e) = cache_entry {
                                new_tile_cache.insert(e.0, e.1);
                                rasterized += 1;
                            } else {
                                cache_hits += 1;
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
            log::warn!(
                concat!(
                    "[pipeline] hover stage 6 rasterize ",
                    $label,
                    " {:>6.1}ms  ({} rasterized, {} hits)"
                ),
                t.elapsed().as_secs_f64() * 1000.0,
                rasterized,
                cache_hits
            );
            (tiles, new_tile_cache)
        }};
    }

    #[cfg(feature = "backend_cairo")]
    let (baked_tiles, new_tile_cache) = {
        use gosub_renderer_cairo::CairoRasterizer;
        rasterize_parallel!(CairoRasterizer::new(), "(cairo):")
    };

    #[cfg(all(feature = "backend_skia", not(feature = "backend_cairo")))]
    let (baked_tiles, new_tile_cache) = {
        use gosub_renderer_skia::SkiaRasterizer;
        rasterize_parallel!(SkiaRasterizer::new(1.0), "(skia): ")
    };

    #[cfg(all(
        feature = "backend_vello",
        not(feature = "backend_cairo"),
        not(feature = "backend_skia")
    ))]
    let (baked_tiles, new_tile_cache) = {
        // Vello: sequential rasterization, no tile-pixel cache yet.
        use gosub_render_pipeline::common::media::MediaStore;
        use gosub_render_pipeline::common::texture_store::TextureStore;
        use gosub_render_pipeline::rasterizer::Rasterable;
        use gosub_renderer_vello::VelloRasterizer;
        let t = Instant::now();
        let ts6 = timing_start!("pipeline.hover.rasterize");
        let media_store = MediaStore::new();
        let mut texture_store = TextureStore::new();
        let mut tiles: Vec<BakedTile> = Vec::new();
        if let Some(ref resources) = _vello_resources {
            let rasterizer = VelloRasterizer::new(std::sync::Arc::clone(resources));
            for &layer_id in &layer_ids {
                for tile_id in tile_list.get_intersecting_tiles(layer_id, full_page_rect) {
                    if let Some(tile) = tile_list.get_tile_mut(tile_id) {
                        if tile.state == TileState::Dirty {
                            if let Some(tid) = rasterizer.rasterize(tile, &mut texture_store, &media_store) {
                                tile.texture_id = Some(tid);
                                tile.state = TileState::Clean;
                            } else {
                                tile.state = TileState::Empty;
                            }
                        }
                    }
                }
            }
            for tile in tile_list.arena.values() {
                if let (Some(tid), true) = (tile.texture_id, tile.state == TileState::Clean) {
                    if let Some(tex) = texture_store.get(tid) {
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
        }
        timing_stop!(ts6);
        log::info!(
            "[pipeline] hover stage 6 rasterize (vello): {:>6.1}ms",
            t.elapsed().as_secs_f64() * 1000.0
        );
        (tiles, std::collections::HashMap::new())
    };

    #[cfg(not(any(feature = "backend_cairo", feature = "backend_skia", feature = "backend_vello")))]
    let (baked_tiles, new_tile_cache): (
        Vec<BakedTile>,
        std::collections::HashMap<TileCacheKey, (u32, u32, Arc<Vec<u8>>)>,
    ) = (Vec::new(), std::collections::HashMap::new());

    // Merge: newly rasterized hover tiles + clean tiles carried from previous render.
    let all_baked_tiles: Vec<BakedTile> = baked_tiles.into_iter().chain(clean_baked).collect();

    log::warn!(
        "[pipeline] hover repaint total: {:.1}ms  ({} total tiles, {} dirty+rasterized)",
        t_total.elapsed().as_secs_f64() * 1000.0,
        all_baked_tiles.len(),
        painted_tiles,
    );

    let cached_tiles = Arc::new(
        all_baked_tiles
            .iter()
            .map(|t| gosub_render_pipeline::render::backend::CachedTile {
                page_x: t.page_x as f32,
                page_y: t.page_y as f32,
                width: t.width,
                height: t.height,
                data: Arc::clone(&t.data),
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
