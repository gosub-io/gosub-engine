//! [`BrowsingContext`]: the runtime state for a single tab's document and rendering -
//! the parsed DOM, viewport, dirty-flag tracking, storage handles, and the pipeline
//! caches (tiles, render list, GPU scene) built from them.
//!
//! Loading itself lives in the tab worker; the worker hands a parsed document to the
//! context via `set_document`, after which the context rebuilds whichever render
//! representation the active backend consumes.

use crate::engine::damage::{Damage, DamageLevel};
use crate::engine::events::{CursorShape, HitTestResponse};
pub use crate::engine::form::Submission;
pub use crate::engine::input::PickerRequest;
use crate::engine::input::{self, InputHost, PageInput};

/// How long a landed image waits for the next before the page re-renders.
#[cfg(all(feature = "process-isolation", target_os = "linux"))]
const REMOTE_MEDIA_SETTLE: std::time::Duration = std::time::Duration::from_millis(150);
use crate::engine::storage::{StorageArea, StorageHandles};
use crate::html::{is_text_input, EngineDocument};
use gosub_config::{Config, HasConfig};
use gosub_render_pipeline::rasterizer::{
    collect_placed_gpu_tiles, cpu_cached_tiles, rasterize_parallel, rasterize_sequential, BakedTile, RasterStrategy,
    Rasterable, TilePixelCache,
};
use gosub_render_pipeline::render::{Color, DisplayItem, RenderContext, RenderList, Viewport};
use gosub_render_pipeline::tile_budget::TileBudget;
use std::sync::Arc;

use crate::html::RenderConfiguration;
use gosub_css3::media_query::{ColorScheme, MediaEnvironment, MediaType, ReducedMotion};
use gosub_interface::css3::{CssSystem, HoverFingerprints};
use gosub_interface::document::Document as _;
use gosub_interface::node::NodeType;
use gosub_render_pipeline::common::browser_state::{BrowserState, WireframeState};
use gosub_render_pipeline::common::document::pipeline_doc::{GosubDocumentAdapter, PipelineDocument};
// The form tests reach it through `super::*`; hit testing itself moved to `input`.
#[cfg(test)]
use gosub_render_pipeline::common::document::pipeline_doc::pseudo_owner;
use gosub_render_pipeline::common::geo::{Dimension as PipelineDimension, Rect as PipelineRect};
use gosub_render_pipeline::common::media::MediaStore;
use gosub_render_pipeline::common::texture::TilePixels;
use gosub_render_pipeline::layering::layer::{LayerId, LayerList};
use gosub_render_pipeline::layouter::taffy::TaffyLayouter;
use gosub_render_pipeline::layouter::{CanLayout, LayoutElementId, LayoutTree};
use gosub_render_pipeline::painter::{PaintScene, Painter};
use gosub_render_pipeline::render::backend::{anchored_tile_pos, CachedTile, ExternalHandle};
use gosub_render_pipeline::rendertree_builder::RenderTree;
use gosub_render_pipeline::tile_budget::defer_tiles_outside_window;
use gosub_render_pipeline::tiler::{TileList, TileState};
use gosub_shared::node::NodeId;
use gosub_shared::{timing_start, timing_stop};
use std::any::Any;
use url::Url;

#[cfg(test)]
// Same waiver as `mod tests` at the bottom of this file: `clippy.toml` exempts the other panics in
// tests, but there is no `allow-unreachable-in-tests` to match.
#[allow(clippy::unreachable)]
mod forms_tests;

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
    /// The root box's width, which bounds horizontal scrolling. Kept here rather than read
    /// from `layer_list`, which a page rendered out of process does not have.
    page_width: f64,
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
    /// Where a remotely rendered page's `#fragment` targets are. Empty for
    /// local renders, which find them through `layer_list`.
    fragment_targets: Vec<crate::fork_server::protocol::FragmentTarget>,
    /// The tile grid stages 4-6 ran against. Its geometry depends only on the layer list and
    /// the tile size, so the raster-window extension resets the per-tile state and reuses it
    /// rather than tiling the page again; every other path replaces it. `None` for a page
    /// rendered out-of-process, which has no local layer list to tile.
    tile_list: Option<TileList>,
    /// Rasterized tile data keyed by (page_x, page_y, layer_id, content_hash).
    /// Passed to the next render so unchanged tiles skip rasterization.
    /// Value is (physical_width, physical_height, pixel_data).
    tile_pixel_cache: TilePixelCache,
}

/// The layouter and the layout tree it produced, kept across frames.
///
/// About half of layout time goes into *building* the taffy tree rather than computing with it
/// (36 ms of 74 ms on a Wikipedia article), and a viewport resize changes none of its inputs:
/// same nodes, same styles - percentages and `auto` reach taffy unresolved, and a sheet using
/// `vw`/`vh` forces a restyle instead (see `style_environment_fingerprint`) - and the same
/// intrinsic sizes. So a resize re-runs taffy over the tree that is already there.
///
/// Dropped whenever the tree itself must be rebuilt: a restyle, a new document, or an image
/// whose intrinsic size arrives after the tree was generated.
struct RetainedLayout {
    layouter: TaffyLayouter,
    layout_tree: Arc<LayoutTree>,
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
    /// What changed since the last frame and how much of the pipeline that invalidates.
    /// Replaces the former cluster of whole-document dirty booleans; see [`Damage`].
    damage: Damage,
    /// Viewport size (width/height only - scroll offset lives in scroll_x/y)
    viewport: Viewport,
    /// Epoch of the scene, used to determine if the scene has changed
    scene_epoch: u64,
    /// Navigation the pipeline's timings are attributed to, so one tab's numbers do not
    /// land in another's. Set when a navigation's document arrives; `None` before the
    /// first document, where samples stay unattributed rather than being misfiled.
    timing_scope: Option<gosub_shared::timing::ScopeId>,

    /// Current scroll offset in CSS pixels.
    scroll_x: f64,
    scroll_y: f64,
    /// True when only the scroll offset changed (no full re-layout needed).
    scroll_dirty: bool,
    /// True when the scroll moved far enough that the raster window must be extended.
    /// Cheaper than a content rebuild: extending re-uses the cached layout.
    raster_dirty: bool,

    /// Device-pixel ratio the cached tiles were rasterized at, or `None` before the first
    /// render. The DPR lives in a process-wide atomic the host writes directly (page zoom
    /// changes it), so it can move without any command reaching this context - see
    /// `invalidate_raster_if_dpr_changed`.
    cache_dpr: Option<u32>,

    /// Cached rasterized tiles for the full page. Valid until content damage is recorded.
    pipeline_cache: Option<PipelineCache>,
    /// GPU-scene cache (paint commands + layer list) for GPU backends. Mutually exclusive in
    /// practice with `pipeline_cache`: a tab uses one path or the other per its backend.
    scene_cache: Option<SceneCache>,
    /// The DOM node currently under the pointer (for :hover matching).
    hover_leaf: Option<NodeId>,
    /// The layout element currently under the pointer, used for bounding-box pre-check.
    hover_layout_element: Option<LayoutElementId>,
    /// Cached :hover fingerprints for the current document; rebuilt on document change.
    hover_fingerprints: Option<HoverFingerprints>,
    /// The document adapter, and with it the per-node computed-style cache, kept alive across
    /// rebuilds. Rebuilding it per frame threw every cached style away, so a resize restyled
    /// the whole document even when nothing about the cascade had changed. Cleared only when
    /// the document itself changes.
    document_adapter: Option<Arc<GosubDocumentAdapter<C>>>,
    /// Layout state reused across frames; see [`RetainedLayout`].
    retained_layout: Option<RetainedLayout>,
    /// The style environment the currently cached computed styles were produced under
    /// (see `CssSystem::style_environment_fingerprint`). A resize that leaves this unchanged
    /// needs layout but no restyle. `None` before the first frame.
    style_fingerprint: Option<u64>,
    /// True when the last hover chain contained a fingerprint-sensitive node.
    hover_chain_sensitive: bool,
    /// The href of the link currently under the pointer, if any.
    pub hover_link_url: Option<String>,
    /// Cursor shape for what is under the pointer, derived from the hovered node's ancestry.
    hover_cursor: CursorShape,
    /// The last point hit-tested: the point, the scroll it was tested against, and the scene
    /// it was tested in. Asking again with all of those the same can only produce the answer
    /// already held.
    ///
    /// The scene is part of it because the geometry is what a hit test reads. A new document
    /// or a re-layout under a pointer that has not moved answers the same question
    /// differently, and without the epoch the cached answer -- hover styling, cursor shape,
    /// link URL -- would stand until the reader moved the mouse.
    hover_probe: Option<(f64, f64, f64, f64, u64)>,
    /// Last pointer position in viewport px (for wheel routing).
    pointer: Option<(f64, f64)>,
    /// Gestures in progress and what the last one asked of the embedder; see [`PageInput`].
    input: PageInput,
    /// Font system for caret placement when the rasterizer doesn't share one (tests, null
    /// backend): the same default the layouter falls back to, so measurements agree.
    fallback_font_system: std::sync::OnceLock<Arc<parking_lot::Mutex<dyn gosub_interface::font_system::FontSystem>>>,

    /// The active backend's per-tile rasterizer and how to drive it. Built once by the tab
    /// worker from the engine's `RenderBackend` (replacing the former per-backend cfg cascade).
    rasterizer: Option<Box<dyn Rasterable + Send + Sync>>,
    raster_strategy: RasterStrategy,

    /// Media store shared between the layout and rasterization stages. The layouter loads
    /// images/SVGs into it by id; the rasterizer resolves the same ids back. It persists
    /// across renders so paint-only repaints (e.g. hover) still find previously loaded media.
    media_store: std::sync::Arc<MediaStore>,

    /// Where the media store asks for bytes. Held here as well so each navigation can tell
    /// it which document its requests belong to. `None` until the tab wires it up.
    media_source: Option<std::sync::Arc<crate::engine::media_source::EngineMediaSource>>,

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
    loader: std::sync::Arc<dyn crate::net::resource_loader::ResourceLoader>,
    /// The source text of the current document, kept when a renderer process
    /// will re-parse it there. `None` when rendering in-process.
    document_source: Option<std::sync::Arc<str>>,
    /// The current document's URL, whether or not this process parsed it.
    document_url: Option<Url>,
    /// Title and icon URL the renderer reported for the current document,
    /// not yet handed to the tab.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    remote_document_meta: Option<(Option<String>, Option<String>)>,
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

    /// The tab this context renders for, as a display string - sent with each
    /// remote render so the renderer process can name itself after the tab in
    /// `ps`/`pstree`. Empty until a remote renderer is installed.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    remote_tab: String,
    /// The remote page's layers back to front, from its last summary: what
    /// orders tiles gathered over several passes for the compositor.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    remote_layer_order: Vec<u64>,
    /// An incremental exchange (scroll, hover) running on its own thread, so
    /// frames keep compositing the tiles already held; merged by
    /// [`Self::poll_remote_passes`].
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    remote_inflight: Option<InflightPass>,
    /// Which page a remote pass is for. Bumped before anything replaces
    /// the page (a new document, a remote navigation) or lets the renderer
    /// go (the tab closing), and shared with pass threads: one started for
    /// an older value must not touch the pool or the renderer, and its result
    /// is dropped rather than merged into the new page.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    remote_epoch: Arc<std::sync::atomic::AtomicU64>,
    /// The hover changed while a pass was in flight; re-raise it once done.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    remote_hover_pending: bool,
    /// The viewport changed size while a pass was in flight: the latest size
    /// only, laid out once the pass lands. A drag produces many sizes; the
    /// renderer sees the ones it has time for.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    remote_resize_pending: Option<(f64, f64)>,
    /// Input that arrived while a pass was in flight, each with the scroll
    /// offset it was measured against, in order. Pointer moves and wheel
    /// notches coalesce; nothing else does. A keystroke is never dropped.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    remote_input_queue: std::collections::VecDeque<(crate::fork_server::protocol::InputEvent, f64)>,
    /// What input passes asked of the broker, with what produced each, for
    /// the tab worker to judge and act on.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    remote_effects: Vec<(InputProvenance, crate::fork_server::protocol::Effect)>,
    /// Why the last out-of-process render could not happen at all - page
    /// content is never rendered in-process instead; the tab worker takes
    /// this and tells the embedder.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    remote_failure: Option<String>,
    /// Images fetched for the resident renderer in the background; a render
    /// proceeds without them and runs again when they land.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    remote_media: std::sync::Arc<crate::fork_server::client::RemoteMediaCache>,
    /// When the first image of the current batch landed; the re-render waits
    /// a little for the rest, so a page of photographs costs a few renders,
    /// not one per photograph.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    remote_media_landed: Option<std::time::Instant>,
}

/// A scroll or hover exchange the tab is waiting on.
#[cfg(all(feature = "process-isolation", target_os = "linux"))]
struct InflightPass {
    what: PassKind,
    generation: u64,
    /// The scroll position the pass was asked for: what its window covers.
    scroll_y: f64,
    page_url: String,
    rx: std::sync::mpsc::Receiver<(
        anyhow::Result<crate::fork_server::client::RenderedPage>,
        std::time::Duration,
    )>,
}

/// The two ways a tab's renders leave this process - which one applies is the
/// configured font system's confinement tier, decided statically.
#[cfg(all(feature = "process-isolation", target_os = "linux"))]
pub enum RemoteRenderer {
    /// A resident renderer from the engine's pool, one per (zone, site),
    /// forked from the warmed fork server (tier `Full`).
    Resident {
        pool: std::sync::Arc<crate::fork_server::pool::RendererPool>,
        zone: crate::zone::ZoneId,
        tab: crate::tab::TabId,
    },
    /// Fork a throwaway renderer per render from the engine's warmed fork
    /// server (tier `Full`, no pool).
    ForkServer(std::sync::Arc<parking_lot::Mutex<crate::fork_server::client::ForkServer>>),
    /// Spawn a throwaway exec'd renderer per render (tier `FontPathsReadable`:
    /// warming buys nothing when font files stay reachable, and the stack may
    /// not even be constructible in a fork server).
    ExecPerRender,
}

impl<C: RenderConfiguration> BrowsingContext<C> {
    /// A context whose out-of-process renderer gets no resources at all, which is all the
    /// tests need; the tab worker builds its context [`with_loader`](Self::with_loader).
    #[cfg(test)]
    pub(crate) fn new(config_store: Config) -> BrowsingContext<C> {
        Self::with_loader(
            config_store,
            std::sync::Arc::new(crate::net::resource_loader::NoResourceLoader),
        )
    }

    /// Creates a new runtime browsing context, sharing the given per-engine settings store.
    /// An out-of-process renderer's resource requests are answered through `loader`.
    pub(crate) fn with_loader(
        config_store: Config,
        loader: std::sync::Arc<dyn crate::net::resource_loader::ResourceLoader>,
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
            damage: Damage::none(),
            viewport: Viewport::default(),
            scene_epoch: 0,
            timing_scope: None,
            scroll_x: 0.0,
            scroll_y: 0.0,
            scroll_dirty: false,
            raster_dirty: false,
            cache_dpr: None,
            pipeline_cache: None,
            scene_cache: None,
            hover_leaf: None,
            hover_layout_element: None,
            hover_fingerprints: None,
            document_adapter: None,
            retained_layout: None,
            style_fingerprint: None,
            hover_chain_sensitive: false,
            hover_link_url: None,
            hover_cursor: CursorShape::Default,
            hover_probe: None,
            pointer: None,
            input: PageInput::default(),
            fallback_font_system: std::sync::OnceLock::new(),
            rasterizer: None,
            raster_strategy: RasterStrategy::None,
            media_store: std::sync::Arc::new(MediaStore::with_decoder(decoder)),
            media_source: None,
            config_store,
            tile_budget: TileBudget::new(),
            loader,
            document_source: None,
            document_url: None,
            #[cfg(all(feature = "process-isolation", target_os = "linux"))]
            remote_document_meta: None,
            #[cfg(all(feature = "process-isolation", target_os = "linux"))]
            remote_tile_memory: Default::default(),
            #[cfg(all(feature = "process-isolation", target_os = "linux"))]
            remote_renderer: None,
            #[cfg(all(feature = "process-isolation", target_os = "linux"))]
            remote_tab: String::new(),
            #[cfg(all(feature = "process-isolation", target_os = "linux"))]
            remote_layer_order: Vec::new(),
            #[cfg(all(feature = "process-isolation", target_os = "linux"))]
            remote_inflight: None,
            #[cfg(all(feature = "process-isolation", target_os = "linux"))]
            remote_epoch: Default::default(),
            #[cfg(all(feature = "process-isolation", target_os = "linux"))]
            remote_hover_pending: false,
            #[cfg(all(feature = "process-isolation", target_os = "linux"))]
            remote_resize_pending: None,
            #[cfg(all(feature = "process-isolation", target_os = "linux"))]
            remote_input_queue: std::collections::VecDeque::new(),
            #[cfg(all(feature = "process-isolation", target_os = "linux"))]
            remote_effects: Vec::new(),
            #[cfg(all(feature = "process-isolation", target_os = "linux"))]
            remote_failure: None,
            #[cfg(all(feature = "process-isolation", target_os = "linux"))]
            remote_media: Default::default(),
            #[cfg(all(feature = "process-isolation", target_os = "linux"))]
            remote_media_landed: None,
        }
    }

    /// Route this tab's full renders out-of-process. Installed once by the
    /// tab worker; see [`Self::remote_render_active`] for when it engages.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    pub fn set_remote_renderer(&mut self, renderer: RemoteRenderer, tab: String) {
        self.remote_renderer = Some(renderer);
        self.remote_tab = tab;
    }

    /// This tab is closing: let go of whatever renderer process hosts it.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    pub fn release_remote_renderer(&mut self) {
        if let Some(RemoteRenderer::Resident { pool, tab, .. }) = self.remote_renderer.take() {
            // Before the release: a pass thread still on its way to the pool
            // must find its page gone (see `RendererPool::renderer_for_live`).
            self.supersede_remote_passes();
            pool.release(tab);
        }
    }

    /// Whether this tab renders in a renderer process at all: a remote mode
    /// was installed for it (none for a backend that presents a GPU texture,
    /// or a font system that cannot be confined).
    #[allow(clippy::needless_return)] // the cfg arms need explicit returns
    pub fn has_remote_renderer(&self) -> bool {
        #[cfg(all(feature = "process-isolation", target_os = "linux"))]
        {
            return self.remote_renderer.is_some();
        }
        #[cfg(not(all(feature = "process-isolation", target_os = "linux")))]
        {
            return false;
        }
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
    /// Tell the media source which navigation its requests belong to.
    ///
    /// The URL decides the `Referer` and whether a `file://` image may be loaded at all; the
    /// reference is what makes the request visible, since the fetcher attaches a null
    /// observer to a request it cannot place. Called when a navigation commits, before the
    /// document is installed, so the first layout's requests already carry it.
    pub fn set_media_navigation(&self, url: Option<Url>, reference: crate::net::req_ref_tracker::RequestReference) {
        if let Some(source) = &self.media_source {
            source.set_document(url, reference);
        }
    }

    /// Wire the media store to the zone's fetcher. Without this the store has nowhere to ask
    /// for bytes, so a page renders with placeholders and nothing is fetched.
    pub fn set_media_source(&mut self, source: std::sync::Arc<crate::engine::media_source::EngineMediaSource>) {
        self.media_store.set_source(source.clone());
        self.media_source = Some(source);
    }

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
    /// Say on the firehose why a full render is about to happen.
    fn note_invalidate(&self, reason: &str) {
        if !crate::telemetry::enabled() {
            return;
        }
        #[cfg(all(feature = "process-isolation", target_os = "linux"))]
        let tab = self.remote_tab.as_str();
        #[cfg(not(all(feature = "process-isolation", target_os = "linux")))]
        let tab = "";
        crate::telemetry::emit("tab.invalidate", serde_json::json!({ "tab": tab, "reason": reason }));
    }

    pub fn set_document(&mut self, doc: Arc<EngineDocument<C>>, source: Option<std::sync::Arc<str>>) {
        let url = {
            use gosub_interface::document::Document as _;
            doc.url()
        };
        self.replace_document(Some(doc), url, source);
    }

    /// A document this process did not parse: the renderer process will, from
    /// `source`. Nothing here holds a DOM for it.
    pub fn set_document_source(&mut self, url: Url, source: std::sync::Arc<str>) {
        self.replace_document(None, Some(url), Some(source));
    }

    pub fn document_url(&self) -> Option<&Url> {
        self.document_url.as_ref()
    }

    /// Title and icon URL the renderer reported since the last call.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    pub fn take_remote_document_meta(&mut self) -> Option<(Option<String>, Option<String>)> {
        self.remote_document_meta.take()
    }

    fn replace_document(
        &mut self,
        doc: Option<Arc<EngineDocument<C>>>,
        url: Option<Url>,
        source: Option<std::sync::Arc<str>>,
    ) {
        #[cfg(all(feature = "process-isolation", target_os = "linux"))]
        {
            self.supersede_remote_passes();
            self.remote_media.clear();
            self.remote_document_meta = None;
            // Input queued for the page that is going would land on the next.
            self.remote_input_queue.clear();
            self.remote_effects.clear();
        }
        self.note_invalidate("document");
        self.document = doc;
        self.damage.rebuild();
        self.document_url = url;
        self.document_source = source;
        self.pipeline_cache = None;
        self.scene_cache = None;
        self.tile_budget.reset();
        self.raster_dirty = false;
        self.hover_leaf = None;
        self.hover_layout_element = None;
        self.hover_fingerprints = None;
        self.document_adapter = None;
        self.retained_layout = None;
        self.style_fingerprint = None;
        self.hover_chain_sensitive = false;
        self.hover_link_url = None;
        self.hover_cursor = CursorShape::Default;
        self.input.end_drag();
        // Node ids belong to the document that is gone; an answer for the old picker must not
        // land on whatever node the new document gave the same id to.
        self.input.forget_picker();
    }

    /// Drop cached raster output when the device-pixel ratio has moved since it was produced.
    ///
    /// Tile pixel data is sized in *physical* pixels, so a DPR change makes every cached tile
    /// the wrong size for the frame about to be composited. Unlike a viewport change there is
    /// no command to hang this off: the host writes `DEVICE_PIXEL_RATIO` directly (page zoom
    /// does exactly that), so the engine only learns about it when a backend reports the new
    /// value. Without this the cached tiles are handed out stamped with the *new* DPR while
    /// still holding pixels rasterized at the old one, and the host scales them by a
    /// correction they do not match - leaving part of the viewport unpainted.
    ///
    /// Must run before the scroll fast path, which returns cached tiles without consulting the
    /// viewport at all.
    pub fn invalidate_raster_if_dpr_changed(&mut self, dpr: u32) {
        if self.cache_dpr == Some(dpr) {
            return;
        }
        let first_render = self.cache_dpr.is_none();
        self.cache_dpr = Some(dpr);
        if first_render {
            // Nothing cached yet; recording the value is enough.
            return;
        }
        self.pipeline_cache = None;
        self.scene_cache = None;
        // A renderer's tile hashes say nothing about DPR, so the pixels it would
        // answer `TileUnchanged` for were rasterized at the old one.
        #[cfg(all(feature = "process-isolation", target_os = "linux"))]
        self.remote_tile_memory.replace_with(std::iter::empty());
        self.tile_budget.reset();
        self.invalidate_render();
        self.raster_dirty = false;
    }

    /// Update the viewport SIZE. Only triggers a full re-layout when width or height changes.
    /// Scroll offset is managed separately via `set_scroll`.
    pub fn set_viewport(&mut self, vp: Viewport) {
        if self.viewport.width == vp.width && self.viewport.height == vp.height {
            return;
        }
        self.viewport.width = vp.width;
        self.viewport.height = vp.height;
        // A page a resident renderer retains is laid out again there, off
        // the tab thread, while this tab keeps compositing the tiles it
        // holds; nothing here is thrown away.
        #[cfg(all(feature = "process-isolation", target_os = "linux"))]
        if self.try_remote_resize() {
            self.note_invalidate("viewport");
            return;
        }
        self.damage.escalate(self.viewport_change_level());
        self.note_invalidate("viewport");
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
        // A move that leaves the page on the same device pixel is no visible change. Callers
        // on most paths pass whole CSS pixels; the GPU tile path passes exact offsets, and moves
        // in device-pixel steps. Compared as rendered positions, not as a distance: a move of
        // less than a device pixel can still cross a rounding boundary.
        let dpr = self.cache_dpr.unwrap_or(1).max(1) as f64;
        let device = |v: f64| (v * dpr).round();
        if device(self.scroll_x) == device(x) && device(self.scroll_y) == device(y) {
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

    /// The device description that `@media` conditions - and viewport-relative units - resolve
    /// against for this tab. Rebuilt per style pass rather than cached, so a settings change
    /// takes effect on the next render without any invalidation plumbing.
    ///
    /// `device-width`/`device-height` report the viewport: the engine renders into an embedder-
    /// owned surface and is never told the screen size. That makes the legacy `device-*`
    /// features behave like their modern counterparts, which is the right answer for a
    /// maximised window and a harmless one otherwise.
    fn media_environment(&self) -> MediaEnvironment {
        let color_scheme = match self.config_store.get_string("renderer.prefers_color_scheme").as_str() {
            "dark" => ColorScheme::Dark,
            _ => ColorScheme::Light,
        };
        // `light-dark()` resolution and the engine-drawn controls (dropdowns, text fields) read
        // the scheme from process-wide flags rather than through the environment; a change is
        // still a restyle because the scheme is part of `style_environment_fingerprint`.
        let dark = matches!(color_scheme, ColorScheme::Dark);
        gosub_css3::stylesheet::set_prefers_dark(dark);
        gosub_render_pipeline::common::theme::set_dark(dark);
        let reduced_motion = if self.config_store.get_bool("renderer.prefers_reduced_motion") {
            ReducedMotion::Reduce
        } else {
            ReducedMotion::NoPreference
        };
        // The live ratio the rasterizer draws at, which the embedder stores on every scale
        // change. Note this is process-wide today, so `resolution` follows the most recently
        // updated window when several are open at different scales.
        let dpr = gosub_render_pipeline::render::DEVICE_PIXEL_RATIO.load(std::sync::atomic::Ordering::Relaxed);

        MediaEnvironment {
            width: self.viewport.width as f32,
            height: self.viewport.height as f32,
            device_width: self.viewport.width as f32,
            device_height: self.viewport.height as f32,
            device_pixel_ratio: dpr.max(1) as f32,
            media_type: MediaType::Screen,
            color_scheme,
            reduced_motion,
            // Flip to `true` when the JS runtime is wired in (M2), so `@media (scripting)`
            // and the `no-js` class pattern report the truth.
            scripting: false,
        }
    }

    #[inline]
    /// Attribute this context's pipeline timings to `scope` (one navigation).
    pub(crate) fn set_timing_scope(&mut self, scope: Option<gosub_shared::timing::ScopeId>) {
        self.timing_scope = scope;
    }

    pub fn scene_epoch(&self) -> u64 {
        self.scene_epoch
    }

    /// Force a full rebuild on the next frame. Embedders use this when something outside the
    /// engine's knowledge changed; internal callers should record the narrowest [`Damage`] they
    /// can instead.
    pub fn invalidate_render(&mut self) {
        self.damage.rebuild();
    }

    /// The damage a viewport resize causes.
    ///
    /// Boxes always move, so layout is the floor. Styles only go stale when the resize changes
    /// what the cascade would produce - a `@media` condition flipping, or viewport-relative
    /// units resolving differently - which [`Self::style_environment_fingerprint`] detects.
    fn viewport_change_level(&self) -> DamageLevel {
        match (self.style_environment_fingerprint(), self.style_fingerprint) {
            // Same environment: no `@media` condition flipped and no sheet reads the viewport,
            // so every cached computed style is still correct. Nothing that feeds the layout
            // tree changed either - percentages and `auto` reach taffy unresolved - so the tree
            // itself stands and only its geometry has to be recomputed.
            (Some(new), Some(old)) if new == old => DamageLevel::Geometry,
            _ => DamageLevel::Style,
        }
    }

    /// Hash the style-relevant environment for the *current* viewport.
    ///
    /// Installs that environment on the way, because the fingerprint has to be read under the
    /// one the next frame will use; `pipeline_build_cache` installs the same value again
    /// before it computes anything.
    fn style_environment_fingerprint(&self) -> Option<u64> {
        let doc = self.document.as_ref()?;
        gosub_css3::media_query::set_media_environment(self.media_environment());
        <C::CssSystem as CssSystem>::style_environment_fingerprint(doc.stylesheets())
    }

    /// Poll whether a background media fetch (e.g. an image download started during layout) has
    /// completed since the last call. When it has, the cached layout is stale, so mark the render
    /// dirty and report `true` so the caller can also wake its own draw loop. The completion flag
    /// is consumed (cleared) by this call.
    pub fn poll_media_completed(&mut self) -> bool {
        if self.media_store.take_completed() {
            self.note_invalidate("media");
            // The image's intrinsic size may only now be known, so boxes can move - but no
            // selector's answer changed, so cached styles stay valid.
            self.damage.escalate(DamageLevel::Layout);
            // Cached tile pixels must go too. A tile's key hashes the media id and box, not
            // whether the media had loaded, so a tile rasterized before an image with a fixed
            // size arrived keeps its key afterwards and would be reused without the image.
            if let Some(cache) = self.pipeline_cache.as_mut() {
                cache.tile_pixel_cache.clear();
            }
            true
        } else {
            false
        }
    }

    /// The adapter this frame's render tree is built from, with its style cache invalidated to
    /// exactly the extent the accumulated damage calls for.
    ///
    /// The adapter carries the per-node computed-style cache, so it is kept across frames: it
    /// used to be rebuilt on every pass, which threw every cached style away and made a resize
    /// restyle the whole document even when nothing about the cascade had changed. A new
    /// document drops it (see [`Self::set_document`]).
    fn prepare_adapter(&mut self) -> Option<Arc<GosubDocumentAdapter<C>>> {
        let adapter = match &self.document_adapter {
            Some(adapter) => Arc::clone(adapter),
            None => {
                let doc = self.document.as_ref()?;
                let adapter = Arc::new(GosubDocumentAdapter::<C>::new(Arc::clone(doc)));
                self.document_adapter = Some(Arc::clone(&adapter));
                adapter
            }
        };

        Some(adapter)
    }

    /// Drop exactly as much of the cached computed styles as the accumulated damage requires.
    ///
    /// Runs on *every* frame that rebuilds, including the geometry-only path. That path does not
    /// re-read styles for layout - it reuses the taffy tree - but painting still reads them, so
    /// a `:hover` change landing in the same frame as a resize would otherwise repaint from a
    /// stale cache.
    fn invalidate_damaged_styles(&mut self) {
        let Some(adapter) = self.document_adapter.as_ref() else {
            return;
        };
        if self.damage.level().needs_restyle() {
            // The cascade would answer differently now, so nothing cached survives.
            adapter.clear_style_cache();
        } else {
            // Only what the damage names needs re-evaluating; everything else keeps its styles.
            adapter.invalidate_style_for_nodes(self.damage.nodes());
        }
    }

    /// Stages 1-3: produce this frame's layer list, and the page height that falls out of it.
    ///
    /// Two paths. When the damage is only [`DamageLevel::Geometry`] and a layout tree is
    /// retained, taffy is re-run over that tree - skipping the render-tree build and the tree
    /// construction inside layout, which together are the larger half of the pipeline. Anything
    /// stronger rebuilds from the document.
    fn build_layer_list(&mut self, media_env: MediaEnvironment) -> Option<(Arc<LayerList>, f64)> {
        // Install the environment that `@media` conditions and viewport-relative CSS units
        // (vw/vh/vmin/vmax, incl. inside clamp()) resolve against. Must precede parse(), which
        // computes styles for display:none filtering.
        gosub_css3::media_query::set_media_environment(media_env);
        self.invalidate_damaged_styles();

        let vp_dim = if self.viewport.width > 0 && self.viewport.height > 0 {
            Some(PipelineDimension::new(
                self.viewport.width as f64,
                self.viewport.height as f64,
            ))
        } else {
            None
        };

        if !self.damage.level().needs_layout_tree() {
            if let Some(retained) = self.retained_layout.as_mut() {
                let ts2 = timing_start!(gosub_shared::timing::Timing::PipelineLayout);
                // Free while nothing else holds the tree, which is why the caller drops the
                // previous frame's caches first: they are what would otherwise share it.
                let tree = Arc::make_mut(&mut retained.layout_tree);
                retained.layouter.relayout(tree, vp_dim);
                timing_stop!(ts2);

                let layout_tree = Arc::clone(&retained.layout_tree);
                let page_height = layout_tree.root_dimension.height;
                let ts3 = timing_start!(gosub_shared::timing::Timing::PipelineLayering);
                let layer_list = Arc::new(LayerList::new(layout_tree));
                timing_stop!(ts3);
                return Some((layer_list, page_height));
            }
        }

        let adapter = self.prepare_adapter()?;

        // Stage 1: render tree
        let ts1 = timing_start!(gosub_shared::timing::Timing::PipelineRenderTree);
        let mut render_tree = RenderTree::new(adapter);
        if let Err(e) = render_tree.parse() {
            // The layouter tolerates a tree without a root; the frame degrades to empty.
            log::error!("Failed to build render tree: {e}");
        }
        timing_stop!(ts1);

        // Stage 2: layout
        let ts2 = timing_start!(gosub_shared::timing::Timing::PipelineLayout);
        // Share the rasterizer's font system so layout and rendering measure/draw against the
        // same font collection (and it's created once, not per layout pass). Backends without a
        // FontSystem (null, Cairo/Pango) fall back to the layouter's own instance.
        let mut layouter = match self.rasterizer.as_deref().and_then(|r| r.font_system()) {
            Some(font_system) => TaffyLayouter::with_font_system(font_system),
            None => TaffyLayouter::new(),
        };
        // Share the persistent media store so resources loaded during layout are visible to the
        // rasterizer (which resolves them by id). Otherwise every image renders as a placeholder.
        layouter.set_media_store(Arc::clone(&self.media_store));
        let layout_tree = Arc::new(layouter.layout(render_tree, vp_dim, 1.0));
        timing_stop!(ts2);

        let page_height = layout_tree.root_dimension.height;
        self.retained_layout = Some(RetainedLayout {
            layouter,
            layout_tree: Arc::clone(&layout_tree),
        });

        // Stage 3: layering
        let ts3 = timing_start!(gosub_shared::timing::Timing::PipelineLayering);
        let layer_list = Arc::new(LayerList::new(layout_tree));
        timing_stop!(ts3);
        Some((layer_list, page_height))
    }

    /// Full pipeline rebuild (stages 1–6): re-tiles and re-rasterizes the whole page,
    /// carrying over the previous tile-pixel cache, then clears the content dirty flags.
    /// Shared by [`Self::rebuild_pipeline_cache_if_needed`] and
    /// [`Self::rebuild_render_list_if_needed`].
    fn rebuild_full_pipeline(&mut self) {
        #[cfg(all(feature = "process-isolation", target_os = "linux"))]
        if self.remote_render_active() {
            match self.try_remote_pipeline() {
                Ok(()) => {
                    // Every tile is live again; an earlier in-process render may have evicted some.
                    // Remote pages are otherwise unbudgeted: their pixels are shared with the
                    // tile memory that makes re-renders incremental, so evicting here frees nothing.
                    self.tile_budget.note_full_raster();
                    // A resident renderer rasterized only the window around the
                    // viewport; scrolling past it asks for more (`try_remote_scroll`).
                    self.note_rastered_window();
                    self.raster_dirty = false;
                    self.damage = Damage::none();
                    return;
                }
                // Isolation is on: page content does not get to run in this
                // process just because the process meant for it is gone. The
                // tab shows nothing until a render succeeds again; the worker
                // reports the failure. Internal pages are the engine's own and
                // may still render here.
                Err(error) if !self.is_internal_page() => {
                    log::error!("out-of-process render failed ({error}); not rendering this page in-process");
                    self.pipeline_cache = None;
                    self.remote_failure = Some(error);
                    self.raster_dirty = false;
                    self.damage = Damage::none();
                    return;
                }
                Err(error) => {
                    log::warn!("out-of-process render of an internal page failed ({error}); rendering it in-process");
                }
            }
        }
        // `pipeline_build_cache` is synchronous - no await can move this work to another
        // thread mid-flight - so a thread-local scope attributes every span it records,
        // including the rasterizer's (whose timers run on this thread, outside its rayon
        // par_iter), to the navigation that owns the document.
        let _scope = self.timing_scope.map(gosub_shared::timing::enter_scope);

        let media_env = self.media_environment();

        // Drop the previous frame's cache before laying out, keeping only its pixels. It holds
        // the other handle on the retained layout tree, and a geometry-only pass has to be the
        // sole owner or `Arc::make_mut` copies the whole tree instead of reusing it.
        let prev_tile_cache = self
            .pipeline_cache
            .take()
            .map(|mut c| std::mem::take(&mut c.tile_pixel_cache))
            .unwrap_or_default();

        if let Some((layer_list, page_height)) = self.build_layer_list(media_env) {
            self.pipeline_cache = Some(pipeline_build_cache(
                layer_list,
                page_height,
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
        // Everything the damage described has now been redone, and the styles in the cache
        // were computed under this environment.
        self.damage = Damage::none();
        self.style_fingerprint = self.style_environment_fingerprint();
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
            tile_list,
            page_height,
            page_width,
            tile_pixel_cache,
            tiles,
            cached_tiles,
            hit_regions,
            fragment_targets,
        } = old_cache;

        // A remotely rendered page has no local layer list to re-tile from.
        // A resident renderer retains the page and extends the window on
        // request; any other remote render already covers the whole page, so
        // there is nothing to extend. Either way the cache goes back first.
        let Some(tile_list) = tile_list else {
            self.pipeline_cache = Some(PipelineCache {
                tiles,
                page_height,
                page_width,
                cached_tiles,
                layer_list,
                hit_regions,
                fragment_targets,
                tile_list: None,
                tile_pixel_cache,
            });
            // The window is noted when the pass lands (`poll_remote_passes`).
            #[cfg(all(feature = "process-isolation", target_os = "linux"))]
            self.try_remote_scroll();
            self.raster_dirty = false;
            return;
        };

        self.pipeline_cache = Some(pipeline_extend_raster(
            tile_list,
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

    /// Mark every remote pass started so far as being for an older page:
    /// see `remote_epoch`.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    fn supersede_remote_passes(&self) {
        self.remote_epoch.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    }

    /// A remote pass rendered the window around `rendered_at`, where the
    /// viewport was when it was asked for. Record that window, not the one
    /// around the viewport now: it may have moved on while the pass ran, and
    /// then what it moved into still needs rendering. Call after
    /// `note_full_raster`, which the check depends on.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    fn note_pass_window(&mut self, rendered_at: f64) {
        if let Some(cache) = self.pipeline_cache.as_ref() {
            self.tile_budget
                .note_rastered_window(rendered_at, self.viewport.height as f64, cache.page_height);
        }
        self.recheck_viewport();
    }

    /// A resize pass rasterized the viewport alone, at the scroll position it
    /// was asked for. Record that band, not the usual window: a scroll would
    /// otherwise walk into rows the renderer never produced. The margin is
    /// asked for only once the drag pauses - while another size waits, the
    /// next resize replaces every tile anyway, and a scroll pass in between
    /// would cost what the tight band saved.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    fn note_resize_band(&mut self, rendered_at: f64) {
        if let Some(cache) = self.pipeline_cache.as_ref() {
            self.tile_budget.note_rastered_band(
                rendered_at,
                rendered_at + self.viewport.height as f64,
                cache.page_height,
            );
        }
        if self.remote_resize_pending.is_none() {
            self.recheck_viewport();
        }
    }

    /// After a remote pass lands: if the viewport now shows what was never
    /// rastered (or was evicted), ask for the window to be extended.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    fn recheck_viewport(&mut self) {
        let page_height = self.active_page_height().unwrap_or(0.0);
        if self
            .tile_budget
            .needs_rerender(self.scroll_y, self.viewport.height as f64, page_height)
        {
            self.raster_dirty = true;
        }
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

    /// Whether the current document is one of the engine's own pages.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    pub(crate) fn is_internal_page(&self) -> bool {
        self.document_url
            .as_ref()
            .is_some_and(|url| matches!(url.scheme(), "gosub" | "about"))
    }

    /// This tab's loader bound to the document it shows, for one render pass:
    /// every request the pass makes is then judged as that document's, even
    /// one answered after the tab has moved on to loading another.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    fn loader_for_document(&self) -> Arc<dyn crate::net::resource_loader::ResourceLoader> {
        self.loader
            .for_document(self.document_url.as_ref())
            .unwrap_or_else(|| Arc::clone(&self.loader))
    }

    /// The reason the last out-of-process render could not happen, once.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    pub fn take_remote_failure(&mut self) -> Option<String> {
        self.remote_failure.take()
    }

    /// Render the current document in a renderer process and adopt the result
    /// as this tab's pipeline cache. A resident renderer that turns out to be
    /// dead is replaced and the render tried once more; the error is what
    /// stopped the last attempt.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    fn try_remote_pipeline(&mut self) -> Result<(), String> {
        // Before the render takes the renderer: a pass waiting for it must
        // find its page superseded rather than run against the new one.
        self.supersede_remote_passes();
        let (Some(remote), Some(source)) = (&self.remote_renderer, &self.document_source) else {
            return Err("no remote renderer or no document source".into());
        };

        // The document's own URL is the renderer's base for relative
        // subresource URLs; about:blank when it has none.
        let page_url = self
            .document_url
            .as_ref()
            .map(|url| url.to_string())
            .unwrap_or_else(|| "about:blank".to_string());
        let viewport = (self.viewport.width as f64, self.viewport.height as f64);
        let started = std::time::Instant::now();
        // What the in-process pipeline would have found in this process: the
        // document the subresource loads are for, and the user's media
        // preferences, which the renderer has no settings of its own to read.
        let env = self.media_environment();
        crate::fork_server::client::set_media_prefs(crate::fork_server::protocol::MediaPrefs {
            prefers_dark: matches!(env.color_scheme, gosub_css3::media_query::ColorScheme::Dark),
            prefers_reduced_motion: matches!(env.reduced_motion, gosub_css3::media_query::ReducedMotion::Reduce),
        });
        let resources = crate::fork_server::client::TabResources {
            loader: self.loader_for_document(),
            media: Arc::clone(&self.remote_media),
        };
        // The whole exchange blocks on the renderer's socket (and, relaying its
        // subresource requests, on the I/O runtime). Blocking a runtime worker
        // while holding its scheduler core can trap tasks woken into this
        // worker's unstealable LIFO slot - the brokered loader's reply path
        // among them - so hand the core to another thread for the duration.
        let run = || match remote {
            RemoteRenderer::Resident { pool, zone, tab } => {
                let site = url::Url::parse(&page_url)
                    .map(|u| crate::fork_server::site::site_of(&u))
                    .unwrap_or_else(|_| "about:".to_string());
                let renderer = pool.renderer_for(*zone, &site, *tab)?;
                let mut renderer = renderer.lock();
                renderer.navigate(
                    source,
                    &page_url,
                    &self.remote_tab,
                    viewport,
                    self.scroll_y,
                    &resources,
                    &self.remote_tile_memory,
                    self.hover_leaf.map(|id| id.into()),
                )
            }
            RemoteRenderer::ForkServer(server) => server.lock().render_page(
                source,
                &page_url,
                &self.remote_tab,
                viewport,
                &resources,
                &self.remote_tile_memory,
                self.hover_leaf.map(|id| id.into()),
            ),
            RemoteRenderer::ExecPerRender => crate::render_process::client::render_page(
                source,
                &page_url,
                &self.remote_tab,
                viewport,
                &resources,
                &self.remote_tile_memory,
                self.hover_leaf.map(|id| id.into()),
            ),
        };
        let blocking = |f: &dyn Fn() -> anyhow::Result<crate::fork_server::client::RenderedPage>| {
            match tokio::runtime::Handle::try_current() {
                Ok(h) if h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
                    tokio::task::block_in_place(f)
                }
                _ => f(),
            }
        };
        let mut result = blocking(&run);
        if let (Err(e), RemoteRenderer::Resident { .. }) = (&result, remote) {
            // The pool replaces a renderer it finds dead on the next request.
            log::warn!("out-of-process render failed ({e}); retrying in a fresh renderer");
            result = blocking(&run);
        }
        match result {
            Ok(page) => {
                report_remote_pass(
                    "remote.navigate",
                    &self.remote_tab,
                    &page_url,
                    self.scroll_y,
                    &page,
                    started.elapsed(),
                );
                // A pass still in flight belongs to the page this replaced
                // (superseded before the render began).
                self.remote_inflight = None;
                self.remote_hover_pending = false;
                self.remote_resize_pending = None;
                self.adopt_remote_page(page);
                Ok(())
            }
            Err(e) => Err(e.to_string()),
        }
    }

    /// A whole page from a renderer replaces what this tab holds.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    fn adopt_remote_page(&mut self, page: crate::fork_server::client::RenderedPage) {
        // This page's tiles are exactly what came back: what an
        // earlier page of this tab kept cannot help it.
        self.remote_tile_memory
            .replace_with(page.tiles.into_iter().map(kept_tile));
        self.remote_layer_order = page.summary.layer_order.clone();
        self.remote_document_meta = Some((page.summary.title.clone(), page.summary.favicon.clone()));
        let baked = self.remote_tile_memory.baked_tiles(&self.remote_layer_order);
        let cached_tiles = Arc::new(gosub_render_pipeline::rasterizer::cpu_cached_tiles(&baked));
        self.pipeline_cache = Some(PipelineCache {
            tiles: baked,
            page_height: page.summary.page_height,
            page_width: page.summary.page_width,
            cached_tiles,
            layer_list: None,
            hit_regions: page.hit_regions,
            fragment_targets: page.summary.fragment_targets,
            tile_list: None,
            tile_pixel_cache: Default::default(),
        });
    }

    /// The viewport moved on a page a resident renderer retains: fetch what
    /// came into its raster window and merge it into this tab's tiles.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    fn try_remote_scroll(&mut self) -> bool {
        self.try_remote_pass(RemotePass::Scroll)
    }

    /// The pointer moved on a page a resident renderer retains: fetch the
    /// tiles it repainted and merge them.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    fn try_remote_hover(&mut self) -> bool {
        self.try_remote_pass(RemotePass::Hover)
    }

    /// The viewport of a page a resident renderer retains changed size: have
    /// the renderer lay it out again at the new size and ship what changed.
    /// Sizes that arrive while a pass is in flight keep only the latest.
    /// False when this tab holds no remotely rendered page, so the caller
    /// takes the in-process path.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    fn try_remote_resize(&mut self) -> bool {
        if !self.remote_input_available() || !self.holds_remote_page() {
            return false;
        }
        let size = (self.viewport.width as f64, self.viewport.height as f64);
        self.try_remote_pass(RemotePass::Resize(size))
    }

    /// Whether what this tab composites came from a renderer: a page it
    /// adopted, which the renderer still has to answer for.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    fn holds_remote_page(&self) -> bool {
        self.pipeline_cache
            .as_ref()
            .is_some_and(|cache| cache.layer_list.is_none())
    }

    /// Start one incremental exchange with the resident renderer on its own
    /// thread, so this tab keeps compositing what it holds meanwhile; the
    /// result is merged by [`Self::poll_remote_passes`]. One pass at a time:
    /// a hover arriving mid-flight is remembered and issued afterwards, a
    /// scroll is re-checked against the window the pass delivers. False when
    /// this tab has no resident renderer.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    fn try_remote_pass(&mut self, what: RemotePass) -> bool {
        let Some(RemoteRenderer::Resident { pool, zone, tab }) = &self.remote_renderer else {
            return false;
        };
        if self.remote_inflight.is_some() {
            match what {
                RemotePass::Hover => self.remote_hover_pending = true,
                RemotePass::Input(event, scroll_y) => self.queue_remote_input(event, scroll_y),
                RemotePass::Resize(size) => self.remote_resize_pending = Some(size),
                RemotePass::Scroll | RemotePass::Media => {}
            }
            return true;
        }
        let kind = what.kind();
        let Some(page_url) = self.document_url.as_ref().map(|url| url.to_string()) else {
            return false;
        };

        let (pool, zone, tab) = (Arc::clone(pool), *zone, *tab);
        let epoch = Arc::clone(&self.remote_epoch);
        let started_for = epoch.load(std::sync::atomic::Ordering::Acquire);
        let remote_tab = self.remote_tab.clone();
        let resources = crate::fork_server::client::TabResources {
            loader: self.loader_for_document(),
            media: Arc::clone(&self.remote_media),
        };
        let scroll_y = match &what {
            // Measured against the viewport the event was sent for, which the
            // page may have scrolled away from since.
            RemotePass::Input(_, scroll_y) => *scroll_y,
            _ => self.scroll_y,
        };
        let hovered = self.hover_leaf.map(|id| id.into());
        let url = page_url.clone();
        let source = self.document_source.clone();
        let viewport = (self.viewport.width as f64, self.viewport.height as f64);
        // An input or resize pass that lays the page out again ships it by
        // content hash against what this tab holds; the other passes never answer
        // `TileUnchanged`, so they look nothing up.
        let known = match &what {
            RemotePass::Input(..) | RemotePass::Resize(_) => self.remote_tile_memory.clone(),
            _ => crate::fork_server::client::TileMemory::default(),
        };
        let (tx, rx) = std::sync::mpsc::channel();
        // The matching completion event (`remote.<kind>`, with the renderer's own lap
        // times) is reported when the result is merged; this is the live half.
        if crate::telemetry::enabled() {
            crate::telemetry::emit(
                &format!("{}.start", kind.event_kind()),
                serde_json::json!({ "tab": self.remote_tab, "url": page_url }),
            );
        }
        let spawned = std::thread::Builder::new()
            .name("gosub-remote-pass".into())
            .spawn(move || {
                let started = std::time::Instant::now();
                let result = (|| {
                    let site = url::Url::parse(&url)
                        .map(|u| crate::fork_server::site::site_of(&u))
                        .unwrap_or_else(|_| "about:".to_string());
                    let current = || epoch.load(std::sync::atomic::Ordering::Acquire) == started_for;
                    let renderer = pool.renderer_for_live(zone, &site, tab, &current)?;
                    let mut renderer = renderer.lock();
                    // A navigation that took the renderer first has replaced
                    // the page this pass was for: running it now would render
                    // (or, for media, retain) the wrong one.
                    if !current() {
                        anyhow::bail!("superseded by a newer page");
                    }
                    match what {
                        RemotePass::Scroll => renderer.scroll(&remote_tab, scroll_y, &resources, &known),
                        RemotePass::Hover => renderer.hover(&remote_tab, hovered, &resources, &known),
                        RemotePass::Input(event, _) => renderer.input(&remote_tab, scroll_y, event, &resources, &known),
                        RemotePass::Resize(size) => renderer.resize(&remote_tab, size, scroll_y, &resources, &known),
                        RemotePass::Media => {
                            let Some(source) = source.as_deref() else {
                                anyhow::bail!("no document source to render again");
                            };
                            renderer.navigate(
                                source,
                                &url,
                                &remote_tab,
                                viewport,
                                scroll_y,
                                &resources,
                                &known,
                                hovered,
                            )
                        }
                    }
                })();
                let _ = tx.send((result, started.elapsed()));
            });
        if let Err(e) = spawned {
            log::warn!("could not start a remote {} pass: {e}", kind.event_kind());
            report_remote_pass_ended(
                kind.event_kind(),
                &self.remote_tab,
                &page_url,
                "spawn_failed",
                Some(&e.to_string()),
            );
            return false;
        }
        self.remote_inflight = Some(InflightPass {
            what: kind,
            generation: started_for,
            scroll_y,
            page_url,
            rx,
        });
        true
    }

    /// Take in whatever out-of-process work landed: an image the renderer
    /// went without, a finished scroll or hover pass. Called every tick by
    /// the tab worker (cheap when nothing is pending); true when a frame
    /// should follow.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    pub fn poll_remote_passes(&mut self) -> bool {
        use std::sync::mpsc::TryRecvError;

        let mut changed = false;
        // An image the renderer went without has arrived: render again, once
        // the ones landing right behind it have had a moment to land too.
        if self.remote_media.take_completed() {
            self.remote_media_landed.get_or_insert_with(std::time::Instant::now);
        }
        if self
            .remote_media_landed
            .is_some_and(|since| since.elapsed() >= REMOTE_MEDIA_SETTLE)
            && self.remote_inflight.is_none()
        {
            self.remote_media_landed = None;
            self.note_invalidate("remote-media");
            // Off the tab thread where a resident renderer allows; the
            // blocking full render is the fallback for the other modes.
            if !self.try_remote_pass(RemotePass::Media) {
                self.damage.rebuild();
            }
            changed = true;
        }

        let Some(inflight) = self.remote_inflight.as_ref() else {
            return changed;
        };
        let (result, exchange) = match inflight.rx.try_recv() {
            Ok(landed) => landed,
            Err(TryRecvError::Empty) => return changed,
            // The thread died without answering: treat it as a failed pass.
            Err(TryRecvError::Disconnected) => (
                Err(anyhow::anyhow!("the remote pass thread ended silently")),
                std::time::Duration::ZERO,
            ),
        };
        let Some(inflight) = self.remote_inflight.take() else {
            return changed;
        };
        let stale = inflight.generation != self.remote_epoch.load(std::sync::atomic::Ordering::Acquire);

        match result {
            Ok(page) if !stale => {
                // The renderer no longer has this page (replaced after a
                // crash, or past its retained-page limit): only a full render
                // gets the tiles back.
                if matches!(
                    inflight.what,
                    PassKind::Scroll | PassKind::Hover | PassKind::Input(_) | PassKind::Resize
                ) && page.summary.no_page
                {
                    log::warn!("resident renderer has no retained page for this tab; rendering it again");
                    report_remote_pass_ended(
                        inflight.what.event_kind(),
                        &self.remote_tab,
                        &inflight.page_url,
                        "no_page",
                        None,
                    );
                    // Whatever input waited was for that page.
                    self.remote_input_queue.clear();
                    self.damage.rebuild();
                } else if let PassKind::Input(provenance) = inflight.what {
                    report_remote_pass(
                        inflight.what.event_kind(),
                        &self.remote_tab,
                        &inflight.page_url,
                        inflight.scroll_y,
                        &page,
                        exchange,
                    );
                    let relaid = !page.hit_regions.is_empty();
                    let effects = std::mem::take(&mut self.remote_effects);
                    let mut effects = effects;
                    effects.extend(page.effects.iter().cloned().map(|effect| (provenance, effect)));
                    let (regions, page_height, page_width, fragment_targets) = (
                        page.hit_regions.clone(),
                        page.summary.page_height,
                        page.summary.page_width,
                        page.summary.fragment_targets.clone(),
                    );
                    self.merge_remote_pass(page);
                    if relaid {
                        // The page was laid out again: its geometry is new, and a
                        // frame must follow so hit tests stop answering from the old.
                        if let Some(cache) = self.pipeline_cache.as_mut() {
                            cache.hit_regions = regions;
                            cache.page_height = page_height;
                            cache.page_width = page_width;
                            cache.fragment_targets = fragment_targets;
                        }
                        self.recheck_viewport();
                    }
                    self.remote_effects = effects;
                    self.scroll_dirty = true;
                } else if matches!(inflight.what, PassKind::Media | PassKind::Resize) {
                    // A whole page, like a navigate: what came back replaces
                    // this tab's tiles and geometry. A resize ships it by
                    // content hash, so most of it is what the tab already held.
                    report_remote_pass(
                        inflight.what.event_kind(),
                        &self.remote_tab,
                        &inflight.page_url,
                        inflight.scroll_y,
                        &page,
                        exchange,
                    );
                    self.adopt_remote_page(page);
                    self.tile_budget.note_full_raster();
                    if matches!(inflight.what, PassKind::Resize) {
                        self.note_resize_band(inflight.scroll_y);
                    } else {
                        self.note_pass_window(inflight.scroll_y);
                    }
                    self.scroll_dirty = true;
                } else {
                    report_remote_pass(
                        inflight.what.event_kind(),
                        &self.remote_tab,
                        &inflight.page_url,
                        inflight.scroll_y,
                        &page,
                        exchange,
                    );
                    self.merge_remote_pass(page);
                    if matches!(inflight.what, PassKind::Scroll) {
                        // Before the check: a fresh raster makes evicted regions live again.
                        self.tile_budget.note_full_raster();
                        self.note_pass_window(inflight.scroll_y);
                    } else {
                        // A hover renders no new window, but a scroll that
                        // came while it ran was not issued (one pass at a
                        // time): whether that left the viewport short is
                        // only known now.
                        self.recheck_viewport();
                    }
                    // A frame with the merged tiles, even if the view is still.
                    self.scroll_dirty = true;
                }
            }
            // A result for a page this tab has since left: nothing to merge.
            Ok(_) => report_remote_pass_ended(
                inflight.what.event_kind(),
                &self.remote_tab,
                &inflight.page_url,
                "stale",
                None,
            ),
            Err(e) => {
                log::warn!(
                    "out-of-process {} render failed ({e}); rendering this page again",
                    inflight.what.event_kind()
                );
                report_remote_pass_ended(
                    inflight.what.event_kind(),
                    &self.remote_tab,
                    &inflight.page_url,
                    if stale { "stale" } else { "failed" },
                    Some(&format!("{e:#}")),
                );
                if !stale {
                    self.damage.rebuild();
                }
            }
        }
        // A size that changed meanwhile goes first: input that waited was
        // measured against a viewport the page is about to be laid out for.
        // Then input: a keystroke is the user's, a hover is cosmetic and is
        // re-raised afterwards.
        if self.remote_inflight.is_none() {
            if let Some(size) = self.remote_resize_pending.take() {
                // The size is not dropped with the pass: what could not be
                // laid out there is laid out here.
                if !self.try_remote_pass(RemotePass::Resize(size)) {
                    self.damage.escalate(self.viewport_change_level());
                }
            }
        }
        if self.remote_inflight.is_none() {
            if let Some((event, scroll_y)) = self.remote_input_queue.pop_front() {
                self.try_remote_pass(RemotePass::Input(event, scroll_y));
            }
        }
        if self.remote_hover_pending {
            self.remote_hover_pending = false;
            self.damage.escalate(DamageLevel::Paint);
        }
        true
    }

    /// The user acted on a page a resident renderer retains: send the event
    /// there, or queue it behind the pass in flight. False when this tab does
    /// not render through a resident renderer, so the caller handles the
    /// input in-process as before.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    pub fn remote_input(&mut self, event: crate::fork_server::protocol::InputEvent) -> bool {
        if !self.remote_input_available() {
            return false;
        }
        self.try_remote_pass(RemotePass::Input(event, self.scroll_y))
    }

    /// Whether input goes out of process: a resident renderer retains this
    /// tab's page. An exec'd renderer keeps nothing between renders and gets
    /// none.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    pub fn remote_input_available(&self) -> bool {
        matches!(self.remote_renderer, Some(RemoteRenderer::Resident { .. })) && self.remote_render_active()
    }

    /// Queue input behind the pass in flight. Consecutive pointer moves keep
    /// only the last and consecutive wheel notches add up: what matters is
    /// where the pointer is and how far the wheel turned, not every step.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    fn queue_remote_input(&mut self, event: crate::fork_server::protocol::InputEvent, scroll_y: f64) {
        use crate::fork_server::protocol::InputEvent;
        if let Some((last, last_scroll)) = self.remote_input_queue.back_mut() {
            match (last, &event) {
                (InputEvent::PointerMove { x, y }, InputEvent::PointerMove { x: nx, y: ny }) => {
                    (*x, *y) = (*nx, *ny);
                    *last_scroll = scroll_y;
                    return;
                }
                (
                    InputEvent::Wheel { x, y, delta_y },
                    InputEvent::Wheel {
                        x: nx,
                        y: ny,
                        delta_y: nd,
                    },
                ) => {
                    (*x, *y) = (*nx, *ny);
                    *delta_y += nd;
                    *last_scroll = scroll_y;
                    return;
                }
                _ => {}
            }
        }
        self.remote_input_queue.push_back((event, scroll_y));
    }

    /// What input passes asked of the broker since the last call, each with
    /// what produced it. The tab worker judges every one before acting.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    pub fn take_remote_effects(&mut self) -> Vec<(InputProvenance, crate::fork_server::protocol::Effect)> {
        std::mem::take(&mut self.remote_effects)
    }

    /// Fold one pass's tiles and evictions into this tab's remote tile set.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    fn merge_remote_pass(&mut self, page: crate::fork_server::client::RenderedPage) {
        self.remote_tile_memory
            .apply_pass(&page.evicted, page.tiles.into_iter().map(kept_tile));
        if !page.summary.layer_order.is_empty() {
            self.remote_layer_order = page.summary.layer_order;
        }
        let baked = self.remote_tile_memory.baked_tiles(&self.remote_layer_order);
        let Some(cache) = self.pipeline_cache.as_mut() else {
            return;
        };
        cache.cached_tiles = Arc::new(gosub_render_pipeline::rasterizer::cpu_cached_tiles(&baked));
        cache.tiles = baked;
    }

    /// Bring the pipeline cache up to date with whatever damage has accumulated.
    ///
    /// Three tiers, cheapest first:
    /// - **Paint** ([`DamageLevel::Paint`]): reuse the cached layout tree and repaint only the
    ///   tiles the damage rects cover, re-evaluating CSS for the damaged nodes alone. `:hover`
    ///   and `:focus` land here.
    /// - **Raster window**: layout and paint both still hold; the scroll just moved far enough
    ///   that more of the page needs rasterizing.
    /// - **Full pipeline** ([`DamageLevel::Layout`] and above): stages 1-6 over the whole page.
    ///
    /// Shared by [`Self::rebuild_pipeline_cache_if_needed`] and
    /// [`Self::rebuild_render_list_if_needed`] so both backends make the same choice.
    fn refresh_pipeline_cache(&mut self) {
        let level = self.damage.level();
        if level.needs_geometry() {
            self.rebuild_full_pipeline();
        } else if self.raster_dirty {
            self.extend_raster_window();
        } else if level == DamageLevel::Paint {
            self.repaint_damaged();
        }
    }

    /// Paint-only repaint: reuse the cached layout tree, skip stages 1-2, and touch only the
    /// tiles the damage covers. Falls back to a full rebuild when there is no cache to reuse.
    fn repaint_damaged(&mut self) {
        let Some(old_cache) = self.pipeline_cache.take() else {
            self.rebuild_full_pipeline();
            return;
        };
        // A remotely rendered page has no layer list to repaint from. A resident renderer
        // retains the page and repaints for us; a one-shot one renders again instead (see
        // `update_hover`).
        let Some(layer_list) = old_cache.layer_list.clone() else {
            self.pipeline_cache = Some(old_cache);
            self.damage = Damage::none();
            #[cfg(all(feature = "process-isolation", target_os = "linux"))]
            self.try_remote_hover();
            return;
        };
        let PipelineCache {
            page_height,
            tile_pixel_cache: prev_tile_cache,
            tiles: prev_baked_tiles,
            ..
        } = old_cache;

        let damage = self.damage.take();
        self.pipeline_cache = Some(pipeline_repaint_damaged(
            layer_list,
            page_height,
            prev_baked_tiles,
            damage.bounding_rect(),
            damage.nodes(),
            &self.viewport,
            self.rasterizer.as_deref(),
            self.raster_strategy,
            prev_tile_cache,
            self.media_store.clone(),
            self.config_store.get_uint("renderer.tile.size") as f64,
        ));
        self.enforce_tile_budget(false);
    }

    /// Rebuild stages 1-6 (pipeline cache) if content has changed, without building a display
    /// list. Used by TileCache backends (Cairo, Skia, Vello) which composite tiles directly
    /// on the host thread and never consume the render list.
    pub fn rebuild_pipeline_cache_if_needed(&mut self) {
        if self.damage.is_none() && !self.scroll_dirty && !self.raster_dirty {
            return;
        }
        self.refresh_pipeline_cache();
        self.scroll_dirty = false;
        self.scene_epoch = self.scene_epoch.wrapping_add(1);
    }

    /// Build/refresh the device-agnostic render list if needed.
    ///
    /// Content damage goes through [`Self::refresh_pipeline_cache`], which picks the cheapest
    /// tier that covers it; a scroll-only change re-composites the cached tiles at the new
    /// offset with no layout or rasterization work.
    pub fn rebuild_render_list_if_needed(&mut self) {
        if self.damage.is_none() && !self.scroll_dirty && !self.raster_dirty {
            return;
        }

        self.refresh_pipeline_cache();

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
        if self.damage.is_none() && !self.scroll_dirty {
            return;
        }
        // Any content damage rebuilds the whole command list. Paint-level damage could reuse
        // the cached layout the way the tile path does, but a GPU re-paint is cheap and avoids
        // the partial-repaint bookkeeping; revisit if it proves hot.
        if !self.damage.is_none() {
            let media_env = self.media_environment();
            // Release the previous scene's handle on the retained layout tree before laying out.
            // It is the other owner, and a geometry-only pass has to be sole owner or
            // `Arc::make_mut` copies the whole tree instead of reusing it - a copy thrown away
            // moments later when the cache below replaces it. `rebuild_full_pipeline` drops its
            // own cache first for the same reason.
            self.scene_cache = None;
            if let Some((layer_list, page_height)) = self.build_layer_list(media_env) {
                self.scene_cache = Some(pipeline_build_scene(
                    layer_list,
                    page_height,
                    &self.viewport,
                    self.rasterizer.as_deref(),
                    self.media_store.clone(),
                ));
            }
            self.damage = Damage::none();
            self.style_fingerprint = self.style_environment_fingerprint();
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

    /// Page-space top of the element a URL fragment points at, per the HTML "indicated part
    /// of the document": the element whose `id` equals the (percent-decoded) fragment, else
    /// the first `<a name=…>` with that name. An empty fragment or `top` means the top of the
    /// document. `None` when nothing matches or layout has not run yet.
    pub fn fragment_target_y(&self, fragment: &str) -> Option<f64> {
        let decoded = percent_encoding::percent_decode_str(fragment).decode_utf8_lossy();
        if decoded.is_empty() || decoded == "top" {
            return Some(0.0);
        }
        use crate::fork_server::protocol::find_fragment_target;
        let Some(layer_list) = self.active_layer_list() else {
            // A remotely rendered page: no local layout, the renderer's list.
            let targets = &self.pipeline_cache.as_ref()?.fragment_targets;
            return find_fragment_target(targets, &decoded);
        };
        let doc = self.document.as_ref()?;
        find_fragment_target(&crate::html::collect_fragment_targets(layer_list, doc), &decoded)
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
        // With `raster_dirty` the cached tile list is missing tiles this frame needs, and any
        // content damage means the tiles themselves are wrong.
        if !self.scroll_dirty || !self.damage.is_none() || self.raster_dirty {
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

    /// The full page width, the horizontal counterpart of [`Self::page_height`]: the root box's
    /// width from whichever cache is active (0 if not yet rendered). No wider than the
    /// viewport unless the content overflows it.
    pub fn page_width(&self) -> f64 {
        self.scene_cache
            .as_ref()
            .map(|c| c.layer_list.layout_tree.root_dimension.width)
            .or_else(|| self.pipeline_cache.as_ref().map(|c| c.page_width))
            .unwrap_or(0.0)
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

    /// Record paint damage covering the margin boxes of `elements`, so the repaint touches
    /// every tile they overlap and no others.
    fn record_element_damage(&mut self, elements: impl IntoIterator<Item = Option<LayoutElementId>>) {
        let Some(layer_list) = self.active_layer_list() else {
            return;
        };
        let rects: Vec<PipelineRect> = elements
            .into_iter()
            .flatten()
            .filter_map(|lei| layer_list.layout_tree.get_node_by_id(lei))
            .map(|el| {
                let m = el.box_model.margin_box;
                PipelineRect::new(m.x, m.y, m.width, m.height)
            })
            .collect();
        for rect in rects {
            self.damage.add_rect(rect);
        }
    }

    /// Describe what is at viewport point `(vp_x, vp_y)` for a context menu: the nearest
    /// enclosing link, an image at the point, editable-ness, and the hit text node's
    /// content. URLs are resolved against `base`. Read-only: does not touch hover state.
    pub fn hit_test(&self, vp_x: f64, vp_y: f64, base: Option<&Url>) -> HitTestResponse {
        let mut out = self.hit_test_unchecked(vp_x, vp_y, base);
        // What the page names goes to the embedder's "open in new tab", "save link as"
        // and "save image as", which act as the user: only what the page could reach
        // itself, so a remote page's `file:///home/u/.ssh/id_ed25519` is offered for
        // nothing.
        let reachable = |url: &String| {
            base.zip(Url::parse(url).ok())
                .is_some_and(|(base, url)| crate::engine::tab::page_may_navigate(base, &url))
        };
        out.link_url = out.link_url.filter(reachable);
        out.image_url = out.image_url.filter(reachable);
        out
    }

    fn hit_test_unchecked(&self, vp_x: f64, vp_y: f64, base: Option<&Url>) -> HitTestResponse {
        let mut out = HitTestResponse::default();
        #[cfg(all(feature = "process-isolation", target_os = "linux"))]
        if let Some(regions) = self.remote_hit_regions() {
            if let Some(region) = hit_region_at(regions, vp_x, vp_y, self.scroll_x, self.scroll_y) {
                out.link_url = region.link.clone();
                out.image_url = region.image.clone();
                out.is_editable = region.editable;
            }
            return out;
        }
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

    /// The font system text measurements use: the rasterizer's, else a shared default.
    fn font_system(&self) -> Arc<parking_lot::Mutex<dyn gosub_interface::font_system::FontSystem>> {
        if let Some(fs) = self.rasterizer.as_ref().and_then(|r| r.font_system()) {
            return fs;
        }
        self.fallback_font_system
            .get_or_init(|| Arc::new(parking_lot::Mutex::new(gosub_fontmanager::ParleyFontSystem::new())))
            .clone()
    }

    /// The input layer, run against this context as its host. Taken out for the call: the
    /// host borrow is the whole context, and the input state is not part of what a host is.
    /// A panic inside leaves the context with a fresh `PageInput`; nothing catches a panic on
    /// the tab worker's thread, so the tab is gone with it either way.
    fn with_input<R>(&mut self, f: impl FnOnce(&mut PageInput, &mut Self) -> R) -> R {
        let mut input = std::mem::take(&mut self.input);
        let out = f(&mut input, self);
        self.input = input;
        out
    }

    /// Whether the focused element is text-editable (input/textarea/contenteditable).
    pub fn focused_editable(&self) -> bool {
        PageInput::focused_editable(self)
    }

    /// The focused element's link target (`<a href>`), for Enter-to-activate.
    pub fn focused_link(&self) -> Option<String> {
        PageInput::focused_link(self)
    }

    pub fn focused_node(&self) -> Option<NodeId> {
        self.document.as_ref().and_then(|d| d.focused_node())
    }

    /// Move focus to `node` (`None` blurs); `visible` = show the ring. See [`PageInput::set_focus`].
    pub fn set_focus(&mut self, node: Option<NodeId>, visible: bool) -> bool {
        self.with_input(|input, host| input.set_focus(host, node, visible))
    }

    /// Click-to-focus: the nearest focusable element under the point (via `<label>` bindings),
    /// or blur when there is none.
    pub fn focus_at(&mut self, vp_x: f64, vp_y: f64) -> bool {
        self.with_input(|input, host| input.focus_at(host, vp_x, vp_y))
    }

    /// Click activation of what's under the point: picks a dropdown row, opens/closes a
    /// `<select>`, toggles a checkbox / selects a radio. Any click closes an open dropdown.
    pub fn activate_at(&mut self, vp_x: f64, vp_y: f64) -> bool {
        self.with_input(|input, host| input.activate_at(host, vp_x, vp_y))
    }

    /// The picker input activated since the last call, for the tab worker to pass on.
    pub fn take_picker_request(&mut self) -> Option<PickerRequest> {
        self.input.take_picker_request()
    }

    /// The embedder's picker moved to `value`: store it on the input that asked, sanitised for
    /// its kind. False when nothing changed, or no picker is open.
    pub fn set_picker_value(&mut self, value: &str) -> bool {
        self.with_input(|input, host| input.set_picker_value(host, value))
    }

    /// The embedder closed its picker; further answers are ignored until the next request.
    pub fn end_picker(&mut self) {
        self.input.end_picker();
    }

    /// Pointer moved with the button held: follow a slider drag (paint-only) or a textarea
    /// resize (re-layout).
    pub fn drag_move(&mut self, vp_x: f64, vp_y: f64) -> bool {
        self.with_input(|input, host| input.drag_move(host, vp_x, vp_y))
    }

    pub fn end_drag(&mut self) {
        self.input.end_drag();
    }

    pub fn is_resizing(&self) -> bool {
        self.input.is_resizing()
    }

    pub fn pointer(&self) -> Option<(f64, f64)> {
        self.pointer
    }

    /// The submission the last click/Enter asked for, if any (consumed).
    pub fn take_submission(&mut self) -> Option<Submission> {
        self.input.take_submission()
    }

    /// Key press for the focused control: text editing, or Space toggling a checkbox/radio.
    /// Returns whether the key was consumed. Ctrl/Meta chords are left alone.
    pub fn edit_key(&mut self, key: &str, ctrl_or_meta: bool, alt: bool, shift: bool) -> bool {
        self.with_input(|input, host| input.edit_key(host, key, ctrl_or_meta, alt, shift))
    }

    /// Committed text (IME / `TextInput`) into the focused text control.
    pub fn insert_text(&mut self, text: &str) -> bool {
        self.with_input(|input, host| input.insert_text(host, text))
    }

    /// Tab / Shift+Tab: next/previous element in tab order, wrapping.
    pub fn focus_step(&mut self, backwards: bool) -> bool {
        self.with_input(|input, host| input.focus_step(host, backwards))
    }

    /// Pointer over an open dropdown: light-highlight the row under it (paint-only).
    pub fn popup_hover_at(&mut self, vp_x: f64, vp_y: f64) -> bool {
        PageInput::popup_hover_at(self, vp_x, vp_y)
    }

    /// Mouse wheel over an open dropdown: scroll its list one row per notch (paint-only).
    pub fn popup_scroll(&mut self, vp_x: f64, vp_y: f64, delta_y: f64) -> bool {
        PageInput::popup_scroll(self, vp_x, vp_y, delta_y)
    }

    /// Wheel over a textarea whose rows overflow scrolls it by `delta_y` px (~3 rows per notch
    /// of 120). Returns whether the wheel was consumed.
    pub fn area_scroll(&mut self, vp_x: f64, vp_y: f64, delta_y: f64) -> bool {
        PageInput::area_scroll(self, vp_x, vp_y, delta_y)
    }

    /// Text the page wants on the clipboard (Ctrl+C / Ctrl+X), once.
    pub fn take_clipboard_write(&mut self) -> Option<String> {
        self.input.take_clipboard_write()
    }

    /// Whether the page asked for a paste (Ctrl+V) since the last call; answer with the
    /// clipboard text as `TextInput`.
    pub fn take_paste_request(&mut self) -> bool {
        self.input.take_paste_request()
    }

    /// What the mouse cursor should be at a viewport point; see [`PageInput::cursor_at`].
    pub fn cursor_at(&self, vp_x: f64, vp_y: f64) -> CursorShape {
        self.input.cursor_at(self, vp_x, vp_y)
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
        let _t_total = gosub_shared::timing_guard!(gosub_shared::timing::Timing::HoverTotal);
        self.pointer = Some((vp_x, vp_y));

        let (scroll_x, scroll_y) = (self.scroll_x, self.scroll_y);

        // The same point as last time cannot hover anything new, and a hit test is not free.
        // Embedders send more of these than one might expect: a windowing system reports
        // motion when the thing under a still pointer changes, so a page that keeps painting
        // keeps asking. Scrolling moves the document under the cursor, so that counts as a
        // move even when the pointer has not.
        let probe = (vp_x, vp_y, scroll_x, scroll_y, self.scene_epoch);
        if self.hover_probe == Some(probe) {
            return (false, false, self.hover_link_url.clone());
        }
        self.hover_probe = Some(probe);

        // A remotely rendered page carries hit-test geometry instead of a layer
        // list; the layout element id is unavailable there, which costs hover
        // repaint, not hit testing (see `PipelineCache::hit_regions`).
        #[cfg(all(feature = "process-isolation", target_os = "linux"))]
        if let Some(regions) = self.remote_hit_regions() {
            let hit = hit_region_at(regions, vp_x, vp_y, scroll_x, scroll_y).cloned();
            return self.apply_remote_hover(hit.as_ref());
        }

        let (new_leaf, new_lei) = {
            let _t = gosub_shared::timing_guard!(gosub_shared::timing::Timing::HoverHitTest);
            input::hit_at(
                self.active_layer_list().map(Arc::as_ref),
                (scroll_x, scroll_y),
                vp_x,
                vp_y,
            )
        };

        self.apply_hover(new_leaf, new_lei)
    }

    /// Hit-test geometry for a remotely rendered page, when that is how the
    /// current page was produced.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    fn remote_hit_regions(&self) -> Option<&[crate::fork_server::protocol::HitRegion]> {
        let cache = self.pipeline_cache.as_ref()?;
        (cache.layer_list.is_none() && !cache.hit_regions.is_empty()).then_some(cache.hit_regions.as_slice())
    }

    /// Hover over a remotely rendered page: the region carries what the
    /// renderer resolved (link, cursor); the renderer's own `Hover` pass does
    /// the restyle and repaint, so any change of element is visually dirty.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    fn apply_remote_hover(
        &mut self,
        hit: Option<&crate::fork_server::protocol::HitRegion>,
    ) -> (bool, bool, Option<String>) {
        use crate::fork_server::protocol::HitCursor;
        let new_leaf = hit.map(|r| NodeId::from(r.node_id));
        if new_leaf == self.hover_leaf {
            return (false, false, self.hover_link_url.clone());
        }
        self.hover_leaf = new_leaf;
        self.hover_layout_element = None;
        let link = hit.and_then(|r| r.link.clone());
        self.hover_cursor = match hit.map(|r| r.cursor) {
            Some(HitCursor::Pointer) => CursorShape::Pointer,
            Some(HitCursor::Text) => CursorShape::Text,
            _ => CursorShape::Default,
        };
        let url_changed = link != self.hover_link_url;
        self.hover_link_url = link.clone();
        // A resident renderer retains the page: paint damage sends it a `Hover`
        // pass (see `repaint_damaged`). A one-shot renderer re-parses per render
        // and skips tiles whose painted content did not change, so a hover
        // re-render stays cheap; anything from Geometry up goes back to it.
        if matches!(self.remote_renderer, Some(RemoteRenderer::Resident { .. })) {
            self.damage.escalate(DamageLevel::Paint);
        } else {
            self.damage.escalate(DamageLevel::Style);
        }
        (true, url_changed, link)
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

        let old_lei = self.hover_layout_element;

        // Collect old and new ancestor chains - only these nodes need CSS cache invalidation.
        // Held locally until it is clear the change is visually significant, since a pointer
        // crossing elements that no `:hover` rule targets must record no damage at all.
        let mut dirty_nodes: Vec<NodeId> = Vec::new();
        if let Some(doc) = &self.document {
            let mut seen = std::collections::HashSet::new();
            for start in [self.hover_leaf, new_leaf].into_iter().flatten() {
                let mut id = start;
                loop {
                    if seen.insert(id) {
                        dirty_nodes.push(id);
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
                let _t = gosub_shared::timing_guard!(gosub_shared::timing::Timing::HoverAncestorWalk);
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
                let _t = gosub_shared::timing_guard!(gosub_shared::timing::Timing::HoverSetHovered);
                doc.set_hovered_nodes(new_leaf);
            }
            // Hover changes only paint (colour, background, outline): the boxes do not move,
            // so record paint-level damage over the old and new hovered elements and let the
            // pipeline repaint just those tiles - in-process, or in the resident renderer that
            // retains the page. A one-shot remote renderer has nothing to repaint from, so
            // hover there renders again; the renderer skips tiles with unchanged content, so
            // this stays cheap.
            #[cfg(all(feature = "process-isolation", target_os = "linux"))]
            let rerender =
                self.remote_render_active() && !matches!(self.remote_renderer, Some(RemoteRenderer::Resident { .. }));
            #[cfg(not(all(feature = "process-isolation", target_os = "linux")))]
            let rerender = false;
            if rerender {
                self.damage.escalate(DamageLevel::Style);
            } else {
                self.damage.escalate(DamageLevel::Paint);
                self.damage.add_nodes(dirty_nodes);
                self.record_element_damage([old_lei, new_lei]);
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

impl<C: RenderConfiguration> InputHost for BrowsingContext<C> {
    type Config = C;

    fn document(&self) -> Option<Arc<EngineDocument<C>>> {
        self.document.clone()
    }

    fn layer_list(&self) -> Option<Arc<LayerList>> {
        self.active_layer_list().cloned()
    }

    fn scroll(&self) -> (f64, f64) {
        (self.scroll_x, self.scroll_y)
    }

    fn viewport_height(&self) -> f64 {
        self.viewport.height as f64
    }

    fn font_system(&self) -> Arc<parking_lot::Mutex<dyn gosub_interface::font_system::FontSystem>> {
        BrowsingContext::font_system(self)
    }

    fn hover_has_link(&self) -> bool {
        self.hover_link_url.is_some()
    }

    fn hover_cursor(&self) -> CursorShape {
        self.hover_cursor
    }

    fn repaint_elements(&mut self, elements: &[Option<LayoutElementId>]) {
        self.damage.escalate(DamageLevel::Paint);
        self.record_element_damage(elements.iter().copied());
    }

    fn damage_nodes(&mut self, nodes: &[NodeId]) {
        for &id in nodes {
            self.damage.add_node(id);
        }
    }

    fn relayout(&mut self) {
        self.invalidate_render();
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

/// GPU-scene build: a paint pass over every element in `layer_list`, producing one ordered
/// paint-command list for the whole page. Skips tiling, rasterization, and compositing - the
/// backend renders the commands into a GPU texture.
///
/// Stages 1-3 happen in [`BrowsingContext::build_layer_list`], which is where the retained
/// layout tree lives.
fn pipeline_build_scene(
    layer_list: Arc<LayerList>,
    page_height: f64,
    viewport: &Viewport,
    rasterizer: Option<&(dyn Rasterable + Send + Sync)>,
    media_store: Arc<MediaStore>,
) -> SceneCache {
    // The layout width, which the paint rect below is sized against. Stages 1-3 moved to
    // `build_layer_list`, so it comes off the layer list rather than a layout tree built here.
    let page_width = layer_list.layout_tree.root_dimension.width;

    // Stage 5′: paint every element into one ordered list (no tiling). Paint over the full page
    // so scrolling reveals already-painted content without a rebuild.
    let layer_count = layer_list.layer_ids.read().len();
    // Paint across the full tile-grid width, not the viewport width: the grid's column count
    // comes from the LAYOUT width (`root_dimension.width`), so a viewport narrower than the
    // layout (horizontal overflow, or a not-yet-allocated 0-width viewport) would collapse this
    // rect and leave every column but the first unpainted and unrasterized.
    let full_page_rect = PipelineRect::new(0.0, 0.0, page_width.max(viewport.width as f64), page_height.max(1.0));
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

/// Stages 4-6: tile the layer list, paint the dirty tiles, and rasterize them into the tile
/// cache. Stages 1-3 happen in [`BrowsingContext::build_layer_list`].
#[allow(clippy::too_many_arguments)]
fn pipeline_build_cache(
    layer_list: Arc<LayerList>,
    page_height: f64,
    viewport: &Viewport,
    scroll_y: f64,
    rasterizer: Option<&(dyn Rasterable + Send + Sync)>,
    strategy: RasterStrategy,
    prev_tile_cache: TilePixelCache,
    media_store: Arc<MediaStore>,
    tile_size: f64,
) -> PipelineCache {
    let ts_total = timing_start!(gosub_shared::timing::Timing::PipelineTotal);

    // Stage 4: tiling
    let ts4 = timing_start!(gosub_shared::timing::Timing::PipelineTiling);
    let mut tile_list = TileList::from_arc(layer_list, PipelineDimension::new(tile_size, tile_size));
    let saved_layer_list = Arc::clone(&tile_list.layer_list);
    tile_list.generate();
    timing_stop!(ts4);

    // Park the rest of the page: stages 5 and 6 below only touch dirty tiles.
    // Scrolling past the window's slack re-rasters around the new position.
    defer_tiles_outside_window(&mut tile_list, scroll_y, viewport.height as f64);

    let render_height = page_height;
    let ts5 = timing_start!(gosub_shared::timing::Timing::PipelinePainting);
    // Paint across the full tile-grid width, not the viewport width: the grid's column count
    // comes from the LAYOUT width (`root_dimension.width`), so a viewport narrower than the
    // layout (horizontal overflow, or a not-yet-allocated 0-width viewport) would collapse this
    // rect and leave every column but the first unpainted and unrasterized. Stages 1-3 moved
    // to `build_layer_list`, so the width comes off the tile list's layer list, the way the
    // two incremental paint paths below already take it.
    let page_width = tile_list.layer_list.layout_tree.root_dimension.width;
    let full_page_rect = PipelineRect::new(0.0, 0.0, page_width.max(viewport.width as f64), render_height.max(1.0));
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
            gosub_shared::timing::Timing::PipelineRasterize,
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
        page_width: saved_layer_list.layout_tree.root_dimension.width,
        cached_tiles,
        layer_list: Some(saved_layer_list),
        hit_regions: Vec::new(),
        fragment_targets: Vec::new(),
        tile_list: Some(tile_list),
        tile_pixel_cache: new_tile_cache,
    }
}

/// Extend the raster window after a scroll: reuse the cached `LayerList` (so stages 1–2 are
/// skipped) and raster only the tiles that are newly inside the window. This is what keeps
/// scrolling a long page off the layout path.
#[allow(clippy::too_many_arguments)]
fn pipeline_extend_raster(
    mut tile_list: TileList,
    page_height: f64,
    mut prev_baked_tiles: Vec<BakedTile>,
    viewport: &Viewport,
    scroll_y: f64,
    rasterizer: Option<&(dyn Rasterable + Send + Sync)>,
    strategy: RasterStrategy,
    mut prev_tile_cache: TilePixelCache,
    media_store: Arc<MediaStore>,
    tile_size: f64,
) -> PipelineCache {
    let layer_list = Arc::clone(&tile_list.layer_list);

    // Stage 4: reuse the grid the last pass built. A scroll moves neither a box nor a layer, and
    // the grid is a pure function of the layer list and the tile size, so tiling again would
    // produce the same tiles, the same element-to-tile assignment and the same R-tree - on a
    // 29 000 px article, 1 201 tiles and 36 827 assignments, four fifths of it an outline lookup
    // and an R-tree query per element. Only the per-tile state differs per pass, and the loop
    // below sets it. Anything that can move a box (layout damage, a viewport resize, a DPR
    // change) drops the cache instead, so nothing stale can reach here.
    let ts4 = timing_start!(gosub_shared::timing::Timing::PipelineExtendTiling);
    let dimension = PipelineDimension::new(tile_size, tile_size);
    if tile_list.default_tile_dimension == dimension {
        tile_list.reset_states();
    } else {
        // `renderer.tile.size` changed under us, and the grid is a function of it.
        tile_list = TileList::from_arc(Arc::clone(&layer_list), dimension);
        tile_list.generate();
        // Everything baked under the old size has to go with it. Both carry-over paths below
        // match on the tile's origin - the baked tiles on `(page_x, page_y, layer_id)`, the
        // pixel cache on that plus a hash of the paint commands - and neither says how big the
        // tile was. A tile at the same origin would be marked `Ready` and handed pixels of the
        // previous size, so the new grid would show the old tiling: gaps where a tile grew,
        // overlap where it shrank, and no repaint to correct either.
        prev_baked_tiles = Vec::new();
        prev_tile_cache = TilePixelCache::new();
    }
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

    // Paint across the full tile-grid width, not the viewport width: the grid's column count
    // comes from the LAYOUT width (`root_dimension.width`), so a viewport narrower than the
    // layout (horizontal overflow, or a not-yet-allocated 0-width viewport) would collapse this
    // rect and leave every column but the first unpainted and unrasterized.
    let page_width = tile_list.layer_list.layout_tree.root_dimension.width;
    let full_page_rect = PipelineRect::new(0.0, 0.0, page_width.max(viewport.width as f64), page_height.max(1.0));
    let layer_ids = tile_list.layer_list.layer_ids.read().clone();

    // Stage 5: only the newly in-window tiles are still dirty.
    let ts5 = timing_start!(gosub_shared::timing::Timing::PipelineExtendPainting);
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
            gosub_shared::timing::Timing::PipelineExtendRasterize,
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
        page_width: layer_list.layout_tree.root_dimension.width,
        cached_tiles,
        layer_list: Some(layer_list),
        hit_regions: Vec::new(),
        fragment_targets: Vec::new(),
        tile_list: Some(tile_list),
        tile_pixel_cache: merged_tile_cache,
    }
}

/// The incremental exchanges a resident renderer answers from its retained page.
#[cfg(all(feature = "process-isolation", target_os = "linux"))]
#[derive(Clone)]
enum RemotePass {
    Scroll,
    Hover,
    /// Images the renderer went without have arrived: render the page again
    /// off the tab thread, so the next navigation is not queued behind it.
    Media,
    /// The user acted on the retained page, at the scroll offset the event's
    /// viewport coordinates were measured against.
    Input(crate::fork_server::protocol::InputEvent, f64),
    /// The viewport changed size: lay the retained page out again at it.
    Resize((f64, f64)),
}

#[cfg(all(feature = "process-isolation", target_os = "linux"))]
impl RemotePass {
    fn kind(&self) -> PassKind {
        match self {
            RemotePass::Scroll => PassKind::Scroll,
            RemotePass::Hover => PassKind::Hover,
            RemotePass::Media => PassKind::Media,
            RemotePass::Input(event, _) => PassKind::Input(InputProvenance::of(event)),
            RemotePass::Resize(_) => PassKind::Resize,
        }
    }
}

/// What a pass in flight is, as the poll side needs it: the kind, and for an
/// input pass what produced it, which decides which of its effects are
/// believed.
#[cfg(all(feature = "process-isolation", target_os = "linux"))]
#[derive(Clone, Copy)]
enum PassKind {
    Scroll,
    Hover,
    Media,
    Input(InputProvenance),
    Resize,
}

#[cfg(all(feature = "process-isolation", target_os = "linux"))]
impl PassKind {
    fn event_kind(self) -> &'static str {
        match self {
            PassKind::Scroll => "remote.scroll",
            PassKind::Hover => "remote.hover",
            PassKind::Media => "remote.media",
            PassKind::Input(_) => "remote.input",
            PassKind::Resize => "remote.resize",
        }
    }
}

/// The clipboard chord a key press was, if any: what a clipboard effect
/// from its pass may be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardChord {
    None,
    Copy,
    Cut,
    Paste,
}

/// What produced an input pass: enough for the tab worker to judge the
/// effects that came back. A cursor is believed from a pointer press or
/// move; a clipboard effect only from the chord that asks for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputProvenance {
    /// A pointer press or move, at a position the cursor can be for.
    pub pointer: bool,
    pub chord: ClipboardChord,
}

impl InputProvenance {
    pub fn of(event: &crate::fork_server::protocol::InputEvent) -> Self {
        use crate::engine::events::Modifiers;
        use crate::fork_server::protocol::InputEvent;
        match event {
            InputEvent::PointerDown { .. } | InputEvent::PointerMove { .. } => Self {
                pointer: true,
                chord: ClipboardChord::None,
            },
            InputEvent::KeyDown { key, modifiers } => {
                let held = Modifiers::from_bits_truncate(*modifiers);
                let chord = if held.intersects(Modifiers::CONTROL | Modifiers::META) {
                    match key.as_str() {
                        "c" | "C" => ClipboardChord::Copy,
                        "x" | "X" => ClipboardChord::Cut,
                        "v" | "V" => ClipboardChord::Paste,
                        _ => ClipboardChord::None,
                    }
                } else {
                    ClipboardChord::None
                };
                Self { pointer: false, chord }
            }
            _ => Self {
                pointer: false,
                chord: ClipboardChord::None,
            },
        }
    }
}

/// The terminal event for a remote pass that produced nothing to merge: it failed,
/// came back for a page the tab has left (`stale`), found no retained page, or never
/// started. Pairs with the `remote.<kind>.start` reported when the pass launched, so a
/// subscriber showing passes in flight can clear every one it announced; a merged pass
/// ends with `remote.<kind>` (see [`report_remote_pass`]) instead.
#[cfg(all(feature = "process-isolation", target_os = "linux"))]
fn report_remote_pass_ended(kind: &str, tab: &str, url: &str, outcome: &str, error: Option<&str>) {
    if !crate::telemetry::enabled() {
        return;
    }
    crate::telemetry::emit(
        &format!("{kind}.ended"),
        serde_json::json!({ "tab": tab, "url": url, "outcome": outcome, "error": error }),
    );
}

/// One remote render pass, onto the telemetry firehose: the exchange as the
/// broker saw it, plus the stage costs the renderer reported.
#[cfg(all(feature = "process-isolation", target_os = "linux"))]
fn report_remote_pass(
    kind: &str,
    tab: &str,
    url: &str,
    scroll_y: f64,
    page: &crate::fork_server::client::RenderedPage,
    exchange: std::time::Duration,
) {
    use crate::fork_server::client::PageTile;
    if !crate::telemetry::enabled() {
        return;
    }
    let fresh = page
        .tiles
        .iter()
        .filter(|t| matches!(t, PageTile::Fresh { .. }))
        .count();
    let bytes: usize = page
        .tiles
        .iter()
        .map(|t| match t {
            PageTile::Fresh { mapping, .. } => mapping.as_slice().len(),
            PageTile::Reused { .. } => 0,
        })
        .sum();
    let renderer: serde_json::Map<String, serde_json::Value> = page
        .summary
        .timings_us
        .iter()
        .map(|(name, us)| (name.clone(), serde_json::json!(us)))
        .collect();
    crate::telemetry::emit(
        kind,
        serde_json::json!({
            "tab": tab,
            "url": url,
            "scroll_y": scroll_y,
            "exchange_us": exchange.as_micros() as u64,
            "tiles_fresh": fresh,
            "tiles_reused": page.tiles.len() - fresh,
            "tiles_evicted": page.evicted.len(),
            "bytes_shipped": bytes,
            "page_height": page.summary.page_height,
            "painted_tiles": page.summary.painted_tiles,
            "renderer_us": renderer,
        }),
    );
}

/// A received tile as this tab keeps it: fresh pixels are the renderer's
/// mapped pages (zero-copy), reused ones are what was kept before.
#[cfg(all(feature = "process-isolation", target_os = "linux"))]
fn kept_tile(tile: crate::fork_server::client::PageTile) -> (u64, crate::fork_server::client::KeptTile) {
    use crate::fork_server::client::{KeptTile, PageTile};
    match tile {
        PageTile::Fresh { header, mapping } => (
            header.content_hash,
            KeptTile::from_header(&header, bytes::Bytes::from_owner(mapping)),
        ),
        PageTile::Reused { header, kept } => (header.content_hash, kept),
    }
}

#[cfg(all(feature = "process-isolation", target_os = "linux"))]
/// Which node a point lands on, per a remotely rendered page's geometry.
fn hit_region_at(
    regions: &[crate::fork_server::protocol::HitRegion],
    vp_x: f64,
    vp_y: f64,
    scroll_x: f64,
    scroll_y: f64,
) -> Option<&crate::fork_server::protocol::HitRegion> {
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
            return Some(region);
        }
    }
    None
}

/// Paint-only repaint: skip stages 1-2 (render-tree + layout), reuse the cached `LayerList`,
/// and repaint only the tiles that intersect `damage_rect`. Every other tile is carried over
/// from `prev_baked_tiles` unchanged - no CSS re-evaluation, no re-rasterization.
///
/// `dirty_nodes` are the DOM nodes whose cached styles are stale; only those are re-evaluated,
/// so the rest of a repainted tile keeps its cached CSS. Used by `:hover` and `:focus`, and by
/// anything else that changes appearance without moving a box.
#[allow(clippy::too_many_arguments)]
fn pipeline_repaint_damaged(
    layer_list: Arc<LayerList>,
    page_height: f64,
    prev_baked_tiles: Vec<BakedTile>,
    damage_rect: Option<PipelineRect>,
    dirty_nodes: &[NodeId],
    viewport: &Viewport,
    rasterizer: Option<&(dyn Rasterable + Send + Sync)>,
    strategy: RasterStrategy,
    prev_tile_cache: TilePixelCache,
    media_store: Arc<MediaStore>,
    tile_size: f64,
) -> PipelineCache {
    // Stage 4: tiling — reuse existing LayerList, no layout work.
    let ts4 = timing_start!(gosub_shared::timing::Timing::PipelineHoverTiling);
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

    // Full-page paint rect and back-to-front layer order - used both to re-emit carried tiles in
    // order (below / in the early-return) and by stages 5–6 further down.
    // Paint across the full tile-grid width, not the viewport width: the grid's column count
    // comes from the LAYOUT width (`root_dimension.width`), so a viewport narrower than the
    // layout (horizontal overflow, or a not-yet-allocated 0-width viewport) would collapse this
    // rect and leave every column but the first unpainted and unrasterized.
    let page_width = tile_list.layer_list.layout_tree.root_dimension.width;
    let full_page_rect = PipelineRect::new(0.0, 0.0, page_width.max(viewport.width as f64), page_height.max(1.0));
    let layer_ids = tile_list.layer_list.layer_ids.read().clone();

    // Mark tiles that DON'T intersect the hover region as Clean.  For Clean tiles we
    // carry the previous BakedTile forward; for Dirty tiles we re-evaluate CSS only
    // for the elements they contain (targeted invalidation).
    // Drop the stale styles once, up front, rather than once per overlapping tile: the node
    // set is the same every time round the loop, and doing it here means it still happens when
    // the damage bounds nothing paintable (a focused element with no box, say) and the loop
    // below is skipped entirely.
    layer_list
        .layout_tree
        .render_tree
        .doc
        .invalidate_style_for_nodes(dirty_nodes);

    let mut clean_baked: Vec<BakedTile> = Vec::with_capacity(total_tiles);
    if let Some(damage_rect) = damage_rect {
        for tile in tile_list.arena.values_mut() {
            let tile_rect = tile.rect;
            let overlaps = tile_rect.x < damage_rect.x + damage_rect.width
                && tile_rect.x + tile_rect.width > damage_rect.x
                && tile_rect.y < damage_rect.y + damage_rect.height
                && tile_rect.y + tile_rect.height > damage_rect.y;
            if overlaps {
                // Leave it Dirty so stages 5-6 repaint it. Everything else in the tile keeps
                // its cached CSS - only the damaged nodes were invalidated, above.
                continue;
            }

            tile.state = TileState::Ready;
            let key = (tile_rect.x.to_bits(), tile_rect.y.to_bits(), tile.layer_id.as_u64());
            if let Some(baked) = prev_by_pos.remove(&key) {
                clean_baked.push(baked);
            }
        }
    } else {
        // Nothing localised to repaint - carry every previous tile forward, but re-emit in
        // back-to-front layer order (see order_baked_tiles_by_layer): `into_values()` is
        // unordered and would scramble overlapping-layer compositing.
        let all_tiles = order_baked_tiles_by_layer(&tile_list, &layer_ids, full_page_rect, prev_by_pos);
        let cached_tiles = Arc::new(cpu_cached_tiles(&all_tiles));
        return PipelineCache {
            tiles: all_tiles,
            page_height,
            page_width: layer_list.layout_tree.root_dimension.width,
            cached_tiles,
            layer_list: Some(layer_list),
            hit_regions: Vec::new(),
            fragment_targets: Vec::new(),
            tile_list: Some(tile_list),
            tile_pixel_cache: prev_tile_cache,
        };
    }

    // Stage 5: paint ONLY dirty (hover-affected) tiles. `full_page_rect` and `layer_ids` were
    // computed above (shared with the carry-over ordering).
    let ts5 = timing_start!(gosub_shared::timing::Timing::PipelineHoverPainting);
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
            gosub_shared::timing::Timing::PipelineHoverRasterize,
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
        page_width: layer_list.layout_tree.root_dimension.width,
        cached_tiles,
        layer_list: Some(layer_list),
        hit_regions: Vec::new(),
        fragment_targets: Vec::new(),
        tile_list: Some(tile_list),
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
    let painter = Painter::new(tile_list.layer_list.clone(), rasterizer.and_then(|r| r.font_system()))
        .with_shape_cache(Arc::clone(&tile_list.shape_cache));

    let mut painted = Vec::new();
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
            painted.push(tile_id);
        }
    }
    // A grid can outlive its pass (the scroll/extend path reuses one), and then the commands
    // written here have to be released before the next pass paints. Reporting them is what
    // makes that a walk of these tiles instead of the whole page.
    tile_list.note_painted(painted);
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
    let ts7 = timing_start!(gosub_shared::timing::Timing::PipelineComposite);

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

/// The image decoder this engine should use, if any. A context can be built
/// before the engine starts and resolves the process settings, so the
/// dispatch precondition is checked here too: without it a decoder child is
/// the embedder re-exec'd, per image. Decoding then stays in-process.
#[cfg(feature = "process-isolation")]
fn image_decoder_from(config: &Config) -> Option<std::sync::Arc<dyn gosub_interface::media_decoder::ImageDecoder>> {
    (crate::child_process::was_dispatched() && config.get_bool("security.image_decoder_process"))
        .then(|| std::sync::Arc::new(crate::decoder_process::client::ProcessImageDecoder) as _)
}

#[cfg(not(feature = "process-isolation"))]
fn image_decoder_from(_config: &Config) -> Option<std::sync::Arc<dyn gosub_interface::media_decoder::ImageDecoder>> {
    None
}

#[cfg(test)]
// `clippy.toml` exempts `unwrap`, `expect` and `panic` in tests centrally, but clippy has no
// `allow-unreachable-in-tests` to match, so the one lint that cannot be waived there is waived
// here. Test code asserting an invariant it set up itself is the case those options exist for.
#[allow(clippy::unreachable)]
mod tests {
    use super::parse_clear_color;

    /// A process that never dispatched child roles (this test binary) decodes
    /// in-process even with the decoder setting on: a decoder child would be
    /// this binary re-exec'd.
    #[cfg(feature = "process-isolation")]
    #[test]
    fn an_undispatched_process_gets_no_decoder_child() {
        let config = crate::engine::settings_store::default_config();
        assert!(config.get_bool("security.image_decoder_process"), "on by default");
        assert!(!crate::child_process::was_dispatched());
        assert!(super::image_decoder_from(&config).is_none());
    }

    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    mod remote_passes {
        use super::super::*;
        use crate::engine::settings_store;
        use crate::html::DefaultRenderConfig;

        /// A context waiting on `what`, answered with `page`.
        fn answered(
            what: RemotePass,
            scroll_y: f64,
            page: crate::fork_server::client::RenderedPage,
        ) -> BrowsingContext<DefaultRenderConfig> {
            let mut ctx: BrowsingContext<DefaultRenderConfig> = BrowsingContext::new(settings_store::default_config());
            let (tx, rx) = std::sync::mpsc::channel();
            tx.send((Ok(page), std::time::Duration::ZERO)).unwrap();
            ctx.remote_inflight = Some(InflightPass {
                what: what.kind(),
                generation: ctx.remote_epoch.load(std::sync::atomic::Ordering::Acquire),
                scroll_y,
                page_url: "https://site.test/".into(),
                rx,
            });
            ctx
        }

        fn empty_page() -> crate::fork_server::client::RenderedPage {
            crate::fork_server::client::RenderedPage {
                summary: Default::default(),
                tiles: Vec::new(),
                hit_regions: Vec::new(),
                evicted: Vec::new(),
                effects: Vec::new(),
            }
        }

        /// What a renderer that retains no page for the tab answers.
        fn no_page() -> crate::fork_server::client::RenderedPage {
            let mut page = empty_page();
            page.summary.no_page = true;
            page
        }

        /// A scroll that came while a hover pass ran was not issued; when the
        /// hover lands, the viewport that moved past the rastered window
        /// still asks for it.
        #[test]
        fn a_hover_pass_rechecks_a_viewport_that_moved_meanwhile() {
            let tall = || {
                let mut page = empty_page();
                page.summary.page_height = 5000.0;
                page
            };
            let mut ctx = answered(RemotePass::Hover, 0.0, tall());
            // Viewport first: setting it drops the pipeline cache.
            ctx.set_viewport(Viewport {
                x: 0,
                y: 0,
                width: 400,
                height: 600,
            });
            ctx.adopt_remote_page(tall());
            ctx.tile_budget.note_rastered_window(0.0, 600.0, 5000.0);
            ctx.scroll_y = 3000.0;
            ctx.raster_dirty = false;
            ctx.poll_remote_passes();
            assert!(
                ctx.raster_dirty,
                "the viewport moved past the rastered window during the hover"
            );
        }

        /// A page rendered out of process has no local layer list; its width comes from the
        /// renderer's summary, or nothing bounds horizontal scrolling on it.
        #[test]
        fn a_remote_page_reports_its_width() {
            let mut page = empty_page();
            page.summary.page_width = 1200.0;
            let mut ctx = answered(RemotePass::Hover, 0.0, empty_page());
            ctx.adopt_remote_page(page);
            assert_eq!(ctx.page_width(), 1200.0);
        }

        /// A pass started for the page a new document replaces is stale the
        /// moment the document is replaced, not only once a render lands:
        /// its media page must not be adopted.
        #[test]
        fn a_new_document_supersedes_a_pass_in_flight() {
            let mut page = empty_page();
            page.summary.page_height = 5000.0;
            let mut ctx = answered(RemotePass::Media, 0.0, page);
            let doc = gosub_html5::html_compile::<DefaultRenderConfig>("<p>the next page</p>");
            ctx.set_document(Arc::new(doc), None);
            ctx.poll_remote_passes();
            assert_ne!(
                ctx.active_page_height(),
                Some(5000.0),
                "the old page's media pass was adopted"
            );
        }

        /// A blank page lays out 0px tall and a hover over it repaints
        /// nothing: an empty answer from a page the renderer does retain,
        /// which must not cost a full render per pointer move.
        #[test]
        fn an_empty_hover_pass_on_a_blank_page_does_not_render_again() {
            let mut ctx = answered(RemotePass::Hover, 0.0, empty_page());
            ctx.poll_remote_passes();
            assert!(
                !matches!(ctx.damage.level(), crate::engine::damage::DamageLevel::Rebuild),
                "{:?}",
                ctx.damage.level()
            );
        }

        /// A media pass rendered the window where the viewport was when it
        /// started; if the viewport moved on meanwhile, the new spot still
        /// needs rendering rather than being taken as rendered.
        #[test]
        fn a_media_pass_records_the_window_it_rendered() {
            let mut page = empty_page();
            page.summary.page_height = 5000.0;
            let mut ctx = answered(RemotePass::Media, 0.0, page);
            ctx.set_viewport(Viewport {
                x: 0,
                y: 0,
                width: 400,
                height: 600,
            });
            ctx.scroll_y = 3000.0;
            ctx.raster_dirty = false;
            ctx.poll_remote_passes();
            assert!(ctx.raster_dirty, "the viewport moved past what the pass rendered");
        }

        /// A resize pass answers with the whole page at its new size, like a
        /// media pass: what came back replaces the tab's geometry.
        #[test]
        fn a_resize_pass_replaces_the_page_geometry() {
            let mut page = empty_page();
            page.summary.page_height = 1899.0;
            let mut ctx = answered(RemotePass::Resize((600.0, 720.0)), 0.0, page);
            ctx.poll_remote_passes();
            assert_eq!(ctx.active_page_height(), Some(1899.0));
            assert!(
                !matches!(ctx.damage.level(), crate::engine::damage::DamageLevel::Rebuild),
                "{:?}",
                ctx.damage.level()
            );
        }

        /// A resize pass rasterizes the viewport alone. Once the drag pauses,
        /// the margin a scroll could reach is asked for.
        #[test]
        fn a_resize_pass_asks_for_the_margin_once_the_drag_pauses() {
            let mut page = empty_page();
            page.summary.page_height = 5000.0;
            let mut ctx = answered(RemotePass::Resize((400.0, 600.0)), 0.0, page);
            ctx.set_viewport(Viewport {
                x: 0,
                y: 0,
                width: 400,
                height: 600,
            });
            ctx.raster_dirty = false;
            ctx.poll_remote_passes();
            assert!(ctx.raster_dirty, "the rows below the viewport were never rastered");
        }

        /// While another size waits, the margin is not asked for: the next
        /// resize replaces every tile, and a scroll pass between the two would
        /// cost what the tight band saved.
        #[test]
        fn a_resize_pass_mid_drag_leaves_the_margin_alone() {
            let mut page = empty_page();
            page.summary.page_height = 5000.0;
            let mut ctx = answered(RemotePass::Resize((400.0, 600.0)), 0.0, page);
            ctx.set_viewport(Viewport {
                x: 0,
                y: 0,
                width: 400,
                height: 600,
            });
            ctx.remote_resize_pending = Some((380.0, 600.0));
            ctx.raster_dirty = false;
            ctx.poll_remote_passes();
            assert!(!ctx.raster_dirty, "a scroll pass was asked for mid-drag");
        }

        /// A renderer that no longer retains the page cannot lay it out at a
        /// new size: the tab renders the page again.
        #[test]
        fn a_resize_pass_without_a_retained_page_renders_the_page_again() {
            let mut ctx = answered(RemotePass::Resize((600.0, 720.0)), 0.0, no_page());
            ctx.poll_remote_passes();
            assert!(
                matches!(ctx.damage.level(), crate::engine::damage::DamageLevel::Rebuild),
                "{:?}",
                ctx.damage.level()
            );
        }

        /// A size that arrived while a pass ran is laid out once the pass
        /// lands, before any input that waited: that input was measured
        /// against the viewport the page is about to be laid out for.
        #[test]
        fn a_pending_resize_is_issued_once_the_pass_lands() {
            let mut ctx = answered(RemotePass::Hover, 0.0, empty_page());
            ctx.remote_resize_pending = Some((600.0, 720.0));
            ctx.poll_remote_passes();
            // No resident renderer here: the pass cannot start, but it was taken.
            assert!(ctx.remote_resize_pending.is_none());
        }

        /// A renderer that no longer retains the page says so for a hover as
        /// for a scroll: the tab renders the page again.
        #[test]
        fn an_empty_hover_pass_renders_the_page_again() {
            let mut ctx = answered(RemotePass::Hover, 0.0, no_page());
            ctx.poll_remote_passes();
            assert!(
                matches!(ctx.damage.level(), crate::engine::damage::DamageLevel::Rebuild),
                "{:?}",
                ctx.damage.level()
            );
        }
    }

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
            ctx.set_document(Arc::new(doc), None);
            ctx.rebuild_pipeline_cache_if_needed();
            ctx
        }

        /// A remotely rendered page has no local layout; the renderer's target
        /// list (the same collector over the same page) resolves the same way.
        #[cfg(all(feature = "process-isolation", target_os = "linux"))]
        #[test]
        fn a_remote_page_resolves_fragments_from_the_renderers_targets() {
            let mut ctx = context_with_targets();
            let targets = {
                let layer_list = ctx.active_layer_list().expect("laid out");
                let doc = ctx.document.as_ref().expect("document");
                crate::html::collect_fragment_targets(layer_list, doc)
            };
            ctx.adopt_remote_page(crate::fork_server::client::RenderedPage {
                summary: crate::fork_server::protocol::PageSummary {
                    fragment_targets: targets,
                    ..Default::default()
                },
                tiles: Vec::new(),
                hit_regions: Vec::new(),
                evicted: Default::default(),
                effects: Vec::new(),
            });
            assert!(ctx.active_layer_list().is_none(), "a remote page keeps no layout");

            let y = ctx.fragment_target_y("section-2").expect("id target");
            assert!((y - 1000.0).abs() < 1.0, "expected ~1000, got {y}");
            let y = ctx.fragment_target_y("legacy%20anchor").expect("name target");
            assert!((y - 1520.0).abs() < 1.0, "expected ~1520, got {y}");
            assert_eq!(ctx.fragment_target_y("nope"), None);
        }

        /// A remotely rendered page has no document to hit-test for the cursor;
        /// the renderer's hit regions say what is under the pointer.
        #[cfg(all(feature = "process-isolation", target_os = "linux"))]
        #[test]
        fn a_remote_page_reports_the_cursor_its_hit_regions_name() {
            use crate::engine::events::CursorShape;
            use crate::fork_server::protocol::{HitCursor, HitRegion, TileWireAnchor};
            let mut ctx: BrowsingContext<DefaultRenderConfig> = BrowsingContext::new(settings_store::default_config());
            ctx.set_viewport(Viewport {
                x: 0,
                y: 0,
                width: 400,
                height: 300,
            });
            let region = |x: f64, cursor: HitCursor, link: Option<&str>| HitRegion {
                x,
                y: 0.0,
                width: 100.0,
                height: 100.0,
                node_id: x as u64 + 1,
                anchor: TileWireAnchor::Scroll,
                link: link.map(str::to_string),
                image: None,
                cursor,
                editable: false,
            };
            ctx.adopt_remote_page(crate::fork_server::client::RenderedPage {
                summary: Default::default(),
                tiles: Vec::new(),
                hit_regions: vec![
                    region(0.0, HitCursor::Pointer, Some("https://example.test/")),
                    region(100.0, HitCursor::Text, None),
                ],
                evicted: Default::default(),
                effects: Vec::new(),
            });
            assert!(ctx.document.is_none(), "a remote page keeps no document");

            ctx.update_hover(50.0, 50.0);
            assert_eq!(ctx.cursor_at(50.0, 50.0), CursorShape::Pointer, "over the link");
            ctx.update_hover(150.0, 50.0);
            assert_eq!(ctx.cursor_at(150.0, 50.0), CursorShape::Text, "over the text");
            ctx.update_hover(350.0, 250.0);
            assert_eq!(ctx.cursor_at(350.0, 250.0), CursorShape::Default, "over nothing");
        }

        /// A local page resolves every target: the payload cap is the
        /// renderer's, not the lookup's.
        #[test]
        fn a_local_page_resolves_targets_past_the_remote_cap() {
            let count = crate::fork_server::protocol::MAX_FRAGMENT_TARGETS + 1;
            let mut ctx: BrowsingContext<DefaultRenderConfig> = BrowsingContext::new(settings_store::default_config());
            ctx.set_viewport(Viewport {
                x: 0,
                y: 0,
                width: 400,
                height: 300,
            });
            let anchors: String = (0..count)
                .map(|i| format!(r#"<a name="t{i}" style="display:block;height:1px"></a>"#))
                .collect();
            let html = format!(r#"<html><body style="margin:0">{anchors}</body></html>"#);
            let mut doc = gosub_html5::html_compile::<DefaultRenderConfig>(&html);
            doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());
            ctx.set_document(Arc::new(doc), None);
            ctx.rebuild_pipeline_cache_if_needed();

            let last = format!("t{}", count - 1);
            let y = ctx.fragment_target_y(&last).expect("the last target resolves");
            assert!((y - (count - 1) as f64).abs() < 1.0, "expected ~{}, got {y}", count - 1);
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
            ctx.set_document(Arc::new(doc), None);
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
            ctx.set_document(Arc::new(doc), None);
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
            // No document URL: nothing to judge the link from, so nothing is offered.
            let hit = ctx.hit_test(50.0, 150.0, None);
            assert_eq!(hit.link_url, None);
            assert_eq!(hit.image_url, None);
        }

        /// A context menu acts as the user: a link or image the page could not follow
        /// itself is not offered to "open in new tab", "save link as" or "save image as".
        #[test]
        fn hit_test_offers_only_what_the_page_could_reach() {
            let mut ctx: BrowsingContext<DefaultRenderConfig> = BrowsingContext::new(settings_store::default_config());
            ctx.set_viewport(Viewport {
                x: 0,
                y: 0,
                width: 400,
                height: 500,
            });
            let html = r#"<html><body style="margin:0">
                <a href="file:///home/u/.ssh/id_ed25519"><img src="file:///etc/passwd" style="display:block;width:100px;height:100px"></a>
                <a href="gosub://settings"><img src="data:image/png;base64,AA" style="display:block;width:100px;height:100px"></a>
            </body></html>"#;
            let mut doc = gosub_html5::html_compile::<DefaultRenderConfig>(html);
            doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());
            ctx.set_document(Arc::new(doc), None);
            ctx.rebuild_pipeline_cache_if_needed();

            let remote = Url::parse("https://evil.test/report.html").unwrap();
            for y in [50.0, 150.0] {
                let hit = ctx.hit_test(50.0, y, Some(&remote));
                assert_eq!(hit.link_url, None, "at {y}");
                assert_eq!(hit.image_url, None, "at {y}");
            }

            // The same file link from a file page is one the page could follow.
            let local = Url::parse("file:///home/u/notes.html").unwrap();
            let hit = ctx.hit_test(50.0, 50.0, Some(&local));
            assert_eq!(hit.link_url.as_deref(), Some("file:///home/u/.ssh/id_ed25519"));
            assert_eq!(hit.image_url.as_deref(), Some("file:///etc/passwd"));
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
            ctx.set_document(Arc::new(doc), None);
            ctx.rebuild_pipeline_cache_if_needed();

            // Tab cycles a → input → button → wraps to a. The tabindex=-1 link is skipped.
            assert!(ctx.focus_step(false));
            let a = ctx.focused_node().expect("first focusable");
            assert_eq!(ctx.focused_link().as_deref(), Some("/one"));
            assert!(ctx.focus_step(false));
            let input = ctx.focused_node().expect("second");
            assert!(ctx.focused_editable());
            assert!(ctx.focus_step(false));
            let button = ctx.focused_node().expect("third");
            assert!(!ctx.focused_editable());
            assert!(ctx.focus_step(false));
            assert_eq!(ctx.focused_node(), Some(a), "wraps around");
            assert!(ctx.focus_step(true));
            assert_eq!(ctx.focused_node(), Some(button), "shift-tab goes back");
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
            ctx.set_document(Arc::new(doc), None);
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

    mod media_queries {
        use super::super::*;
        use crate::engine::settings_store;
        use crate::html::DefaultRenderConfig;
        use gosub_css3::system::Css3System;

        /// A page whose only content is 100px tall below the breakpoint and 1000px tall above
        /// it, so the laid-out page height reports which branch of the `@media` block won.
        fn context_at_width(width: u32) -> BrowsingContext<DefaultRenderConfig> {
            let mut ctx: BrowsingContext<DefaultRenderConfig> = BrowsingContext::new(settings_store::default_config());
            ctx.set_viewport(Viewport {
                x: 0,
                y: 0,
                width,
                height: 300,
            });
            let html = r#"<html><head><style>
                    #box { display: block; height: 100px; }
                    @media (min-width: 600px) { #box { height: 1000px; } }
                </style></head>
                <body style="margin:0"><div id="box"></div></body></html>"#;
            let mut doc = gosub_html5::html_compile::<DefaultRenderConfig>(html);
            doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());
            ctx.set_document(Arc::new(doc), None);
            ctx.rebuild_pipeline_cache_if_needed();
            ctx
        }

        /// The context must feed its own viewport into the media environment, so the same
        /// document lays out differently in a narrow and a wide tab.
        #[test]
        fn viewport_width_selects_the_media_branch() {
            let narrow = context_at_width(400);
            assert!(
                (narrow.page_height() - 100.0).abs() < 1.0,
                "below the breakpoint: expected ~100, got {}",
                narrow.page_height()
            );

            let wide = context_at_width(800);
            assert!(
                (wide.page_height() - 1000.0).abs() < 1.0,
                "above the breakpoint: expected ~1000, got {}",
                wide.page_height()
            );
        }

        /// Resizing an existing tab across the breakpoint must re-resolve styles, not just
        /// re-run layout against the cached ones.
        #[test]
        fn resizing_across_the_breakpoint_restyles() {
            let mut ctx = context_at_width(400);
            assert!((ctx.page_height() - 100.0).abs() < 1.0);

            ctx.set_viewport(Viewport {
                x: 0,
                y: 0,
                width: 800,
                height: 300,
            });
            ctx.rebuild_pipeline_cache_if_needed();
            assert!(
                (ctx.page_height() - 1000.0).abs() < 1.0,
                "after widening: expected ~1000, got {}",
                ctx.page_height()
            );

            // And back again.
            ctx.set_viewport(Viewport {
                x: 0,
                y: 0,
                width: 400,
                height: 300,
            });
            ctx.rebuild_pipeline_cache_if_needed();
            assert!(
                (ctx.page_height() - 100.0).abs() < 1.0,
                "after narrowing: expected ~100, got {}",
                ctx.page_height()
            );
        }
    }

    /// How much of the pipeline each kind of change actually invalidates. These assert on the
    /// recorded [`DamageLevel`] rather than on timings, so they stay meaningful as the
    /// pipeline gets faster.
    mod invalidation {
        use super::super::*;
        use crate::engine::settings_store;
        use crate::html::DefaultRenderConfig;
        use gosub_css3::system::Css3System;

        /// Load `html` at 800x600 and run one full build, so there is a style cache and a
        /// recorded style environment for the next change to be measured against.
        fn built_context(html: &str) -> BrowsingContext<DefaultRenderConfig> {
            let mut ctx: BrowsingContext<DefaultRenderConfig> = BrowsingContext::new(settings_store::default_config());
            ctx.set_viewport(Viewport {
                x: 0,
                y: 0,
                width: 800,
                height: 600,
            });
            let mut doc = gosub_html5::html_compile::<DefaultRenderConfig>(html);
            doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());
            ctx.set_document(Arc::new(doc), None);
            ctx.rebuild_pipeline_cache_if_needed();
            assert!(ctx.damage.is_none(), "the initial build should consume its damage");
            ctx
        }

        fn resize(ctx: &mut BrowsingContext<DefaultRenderConfig>, width: u32) {
            ctx.set_viewport(Viewport {
                x: 0,
                y: 0,
                width,
                height: 600,
            });
        }

        const PLAIN: &str = r#"<html><head><style>
                #target { display: block; width: 100px; height: 50px; }
            </style></head><body><div id="target">x</div></body></html>"#;

        /// The Stage B payoff: a resize that changes nothing the cascade reads needs layout,
        /// not a restyle, so every cached computed style survives it.
        #[test]
        fn resize_without_a_breakpoint_does_not_restyle() {
            let mut ctx = built_context(PLAIN);
            resize(&mut ctx, 900);
            assert_eq!(
                ctx.damage.level(),
                DamageLevel::Geometry,
                "no @media condition flipped and no sheet reads the viewport, so neither the \
                 styles nor the layout tree need rebuilding - only the geometry"
            );
        }

        /// ...but a resize that flips a media condition must restyle, or the page would keep
        /// rendering the wrong branch.
        #[test]
        fn resize_across_a_breakpoint_restyles() {
            let html = r#"<html><head><style>
                    #target { display: block; width: 100px; }
                    @media (min-width: 850px) { #target { width: 300px; } }
                </style></head><body><div id="target">x</div></body></html>"#;
            let mut ctx = built_context(html);

            // 800 -> 820 stays on the same side of the 850px breakpoint.
            resize(&mut ctx, 820);
            assert_eq!(
                ctx.damage.level(),
                DamageLevel::Geometry,
                "same side of the breakpoint: styles and the layout tree both still hold"
            );

            ctx.rebuild_pipeline_cache_if_needed();
            // 820 -> 900 crosses it.
            resize(&mut ctx, 900);
            assert_eq!(ctx.damage.level(), DamageLevel::Style, "the breakpoint flipped");
        }

        /// A sheet using `vw`/`vh` resolves those at style-computation time, so every resize
        /// invalidates its computed values no matter what the media conditions say.
        #[test]
        fn resize_with_viewport_units_always_restyles() {
            let html = r#"<html><head><style>
                    #target { display: block; width: 50vw; }
                </style></head><body><div id="target">x</div></body></html>"#;
            let mut ctx = built_context(html);
            resize(&mut ctx, 900);
            assert_eq!(
                ctx.damage.level(),
                DamageLevel::Style,
                "viewport units make any resize a restyle"
            );
        }

        /// Two resizes before a frame is drawn combine into the stronger of the two, so a
        /// breakpoint crossing cannot be masked by a later harmless resize.
        #[test]
        fn damage_from_several_resizes_combines() {
            let html = r#"<html><head><style>
                    #target { display: block; width: 100px; }
                    @media (min-width: 850px) { #target { width: 300px; } }
                </style></head><body><div id="target">x</div></body></html>"#;
            let mut ctx = built_context(html);

            resize(&mut ctx, 900); // crosses the breakpoint -> Style
            resize(&mut ctx, 810); // back over it; on its own this would be Layout
            assert_eq!(
                ctx.damage.level(),
                DamageLevel::Style,
                "the restyle the first resize needed must not be lost"
            );
        }

        /// Focus used to force a full rebuild, re-rasterizing the whole page on every Tab.
        /// `:focus` cannot move a box and `:focus-within` is not implemented, so it is
        /// paint-level damage over the two elements involved.
        #[test]
        fn focus_change_is_paint_only() {
            let html = r#"<html><head><style>
                    a { display: block; height: 30px; }
                    a:focus { background-color: #ff0000; }
                </style></head><body style="margin:0">
                    <a href="/one">first</a>
                    <a href="/two">second</a>
                </body></html>"#;
            let mut ctx = built_context(html);

            assert!(ctx.focus_step(false));
            let first = ctx.focused_node().expect("a focusable link");
            assert_eq!(
                ctx.damage.level(),
                DamageLevel::Paint,
                "focus must not escalate to a rebuild"
            );
            assert_eq!(ctx.damage.nodes(), &[first], "only the newly focused element is stale");
            assert!(
                ctx.damage.bounding_rect().is_some(),
                "the focused element's box should bound the repaint"
            );

            // Moving on records both the element losing focus and the one gaining it.
            ctx.rebuild_pipeline_cache_if_needed();
            assert!(ctx.focus_step(false));
            let second = ctx.focused_node().expect("a second focusable link");
            assert_eq!(ctx.damage.level(), DamageLevel::Paint);
            assert_eq!(ctx.damage.nodes(), &[first, second]);
        }

        /// An image finishing its decode can move boxes but cannot change what any selector
        /// matches, so it needs layout without a restyle.
        #[test]
        fn decoded_media_needs_layout_but_not_restyle() {
            let mut ctx = built_context(PLAIN);
            ctx.damage.escalate(DamageLevel::Layout);
            // The tree must be rebuilt (the new intrinsic size is baked into it) but the
            // computed styles behind it are untouched.
            assert!(ctx.damage.level().needs_layout_tree());
            assert!(!ctx.damage.level().needs_restyle());
        }

        /// The point of the whole exercise: a focus change must rasterize far fewer tiles than
        /// a full rebuild, *and* still leave a complete tile set behind.
        ///
        /// The second half is the one that bites. A partial path that drops or reorders tiles
        /// looks fine in a damage-level assertion and shows up as blank or corrupted regions on
        /// screen, so this compares the resulting tile set against the full rebuild's.
        #[test]
        fn focus_repaint_touches_few_tiles_and_loses_none() {
            use gosub_render_pipeline::common::texture::TextureId;
            use gosub_render_pipeline::common::texture_store::TextureStore;
            use gosub_render_pipeline::render::backend::PixelFormat;
            use gosub_render_pipeline::tiler::Tile;
            use std::sync::atomic::{AtomicUsize, Ordering};

            struct CountingRasterizer {
                calls: Arc<AtomicUsize>,
            }
            impl Rasterable for CountingRasterizer {
                fn rasterize(&self, tile: &Tile, store: &mut TextureStore, _media: &MediaStore) -> Option<TextureId> {
                    self.calls.fetch_add(1, Ordering::Relaxed);
                    let (w, h) = (tile.rect.width as usize, tile.rect.height as usize);
                    Some(store.add(w, h, vec![0xFFu8; w * h * 4], PixelFormat::PreMulArgb32))
                }
            }

            /// Tile identity: position plus layer, the key the repaint path carries tiles by.
            fn tile_keys(ctx: &BrowsingContext<DefaultRenderConfig>) -> Vec<(u64, u64, u64)> {
                let Some(cache) = ctx.pipeline_cache.as_ref() else {
                    unreachable!("a build must leave a pipeline cache");
                };
                let mut keys: Vec<(u64, u64, u64)> = cache
                    .tiles
                    .iter()
                    .map(|t| (t.page_x.to_bits(), t.page_y.to_bits(), t.layer_id))
                    .collect();
                keys.sort_unstable();
                keys
            }

            // A page several tile rows tall, so "repaint everything" and "repaint one element"
            // are clearly different amounts of work.
            let html = r#"<html><head><style>
                    a { display: block; height: 40px; }
                    a:focus { background-color: #ff0000; }
                </style></head><body style="margin:0">
                    <div style="height:900px;background:#eee"></div>
                    <a href="/one">first</a>
                    <a href="/two">second</a>
                </body></html>"#;

            let calls = Arc::new(AtomicUsize::new(0));
            let mut ctx: BrowsingContext<DefaultRenderConfig> = BrowsingContext::new(settings_store::default_config());
            ctx.set_rasterizer(
                Box::new(CountingRasterizer {
                    calls: Arc::clone(&calls),
                }),
                RasterStrategy::ParallelCached,
            );
            ctx.set_viewport(Viewport {
                x: 0,
                y: 0,
                width: 512,
                height: 1024,
            });
            let mut doc = gosub_html5::html_compile::<DefaultRenderConfig>(html);
            doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());
            ctx.set_document(Arc::new(doc), None);

            ctx.rebuild_pipeline_cache_if_needed();
            let full_rebuild_calls = calls.swap(0, Ordering::Relaxed);
            let full_keys = tile_keys(&ctx);
            assert!(full_rebuild_calls > 4, "the page should span several tiles");

            // Focus the first link and repaint.
            assert!(ctx.focus_step(false));
            assert_eq!(ctx.damage.level(), DamageLevel::Paint);
            ctx.rebuild_pipeline_cache_if_needed();
            let repaint_calls = calls.swap(0, Ordering::Relaxed);

            // Measured 2 of 8 here. The bound is deliberately loose - what it has to catch is
            // a regression back to repainting the whole page, and the repaint cost is set by
            // the focused element's size, so the margin only widens on a real page.
            assert!(
                repaint_calls * 2 <= full_rebuild_calls,
                "a focus change rasterized {repaint_calls} of the full rebuild's \
                 {full_rebuild_calls} tiles - the partial path is not doing its job"
            );
            assert_eq!(
                tile_keys(&ctx),
                full_keys,
                "the repaint must leave exactly the tiles a full rebuild would - a dropped or \
                 duplicated tile shows up as a blank or corrupted band on screen"
            );
        }

        /// A resize must re-run taffy over the tree it already has, not build a new one.
        ///
        /// Identity is the check: the retained `Arc<LayoutTree>` has to be the *same allocation*
        /// afterwards. Comparing contents would pass even if the tree were rebuilt from scratch,
        /// which is exactly the thing this is meant to catch. It also catches `Arc::make_mut`
        /// silently deep-copying the tree because the previous frame's cache was still holding
        /// it - that would turn the optimisation into a pessimisation, and the pointer changes.
        #[test]
        fn resize_reuses_the_layout_tree() {
            let mut ctx = built_context(PLAIN);
            let before = ctx
                .retained_layout
                .as_ref()
                .map(|r| Arc::as_ptr(&r.layout_tree))
                .expect("the first build retains a layout tree");

            resize(&mut ctx, 900);
            assert_eq!(ctx.damage.level(), DamageLevel::Geometry);
            ctx.rebuild_pipeline_cache_if_needed();

            let after = ctx
                .retained_layout
                .as_ref()
                .map(|r| Arc::as_ptr(&r.layout_tree))
                .expect("still retained after the resize");
            assert_eq!(
                before, after,
                "the resize rebuilt the layout tree instead of reusing it"
            );

            // And the geometry really was recomputed against the new width.
            let Some(cache) = ctx.pipeline_cache.as_ref() else {
                unreachable!("a resize must leave a pipeline cache");
            };
            assert!(cache.page_height > 0.0);
        }

        /// ...but anything stronger than `Geometry` must build a fresh tree, because the inputs
        /// it was generated from have changed. An image finishing its decode is the case that
        /// matters: its intrinsic size is baked into the tree at generation time.
        #[test]
        fn stronger_damage_rebuilds_the_layout_tree() {
            let mut ctx = built_context(PLAIN);
            let before = ctx
                .retained_layout
                .as_ref()
                .map(|r| Arc::as_ptr(&r.layout_tree))
                .expect("the first build retains a layout tree");

            ctx.damage.escalate(DamageLevel::Layout);
            ctx.rebuild_pipeline_cache_if_needed();

            let after = ctx
                .retained_layout
                .as_ref()
                .map(|r| Arc::as_ptr(&r.layout_tree))
                .expect("a new tree is retained");
            assert_ne!(
                before, after,
                "Layout-level damage must rebuild the tree - reusing it would keep the stale \\
                 intrinsic sizes that caused the damage in the first place"
            );
        }

        /// A new document invalidates everything, and drops the adapter so no stale computed
        /// style can leak from the previous page.
        #[test]
        fn navigation_rebuilds_and_drops_the_style_cache() {
            let mut ctx = built_context(PLAIN);
            assert!(ctx.document_adapter.is_some(), "the first build creates an adapter");

            let mut doc = gosub_html5::html_compile::<DefaultRenderConfig>(PLAIN);
            doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());
            ctx.set_document(Arc::new(doc), None);

            assert_eq!(ctx.damage.level(), DamageLevel::Rebuild);
            assert!(
                ctx.document_adapter.is_none(),
                "the previous adapter must not be reused"
            );
            assert!(ctx.style_fingerprint.is_none());
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

        #[test]
        fn sequential_rasterization_returns_tiles_in_layer_order() {
            let mut ctx: BrowsingContext<DefaultRenderConfig> = BrowsingContext::new(settings_store::default_config());
            ctx.set_rasterizer(
                Box::new(SolidRasterizer {
                    calls: Arc::new(AtomicUsize::new(0)),
                }),
                RasterStrategy::Sequential,
            );
            ctx.set_viewport(Viewport {
                x: 0,
                y: 0,
                width: 1024,
                height: 1024,
            });
            // Every half-transparent box is a layer of its own, over the page's tiles.
            let boxes: String = (0..8)
                .map(|i| {
                    format!(
                        r#"<div style="opacity:0.5;height:100px;margin-top:{}px;background:red"></div>"#,
                        i * 10
                    )
                })
                .collect();
            let html = format!(r#"<html><body style="margin:0;background:#ddd">{boxes}</body></html>"#);
            let mut doc = gosub_html5::html_compile::<DefaultRenderConfig>(&html);
            doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());
            ctx.set_document(Arc::new(doc), None);

            ctx.rebuild_pipeline_cache_if_needed();

            let Some(cache) = ctx.pipeline_cache.as_ref() else {
                unreachable!("pipeline cache must exist after rebuild");
            };
            let Some(layer_list) = cache.layer_list.as_ref() else {
                unreachable!("a local pass keeps its layer list");
            };
            let order: Vec<u64> = layer_list.layer_ids.read().iter().map(|id| id.as_u64()).collect();
            assert!(
                order.len() > 2,
                "the page must have several layers, got {}",
                order.len()
            );
            let ranks: Vec<usize> = cache
                .tiles
                .iter()
                .map(|t| order.iter().position(|id| *id == t.layer_id).unwrap_or(usize::MAX))
                .collect();
            assert!(
                ranks.windows(2).all(|w| w[0] <= w[1]),
                "tiles must be composited layer by layer, bottom first: {ranks:?}"
            );
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
            ctx.set_document(Arc::new(doc), None);

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
        fn a_scroll_that_lands_on_a_new_device_pixel_is_kept() {
            let (mut ctx, _) = tall_page_context(128);
            ctx.invalidate_raster_if_dpr_changed(3);
            // At 3x, 0.4 CSS px renders at device pixel 1 (1.2) and 0.5 at 2 (1.5): a tenth of a
            // CSS pixel, but the page moves.
            ctx.set_scroll(0.0, 0.4);
            ctx.set_scroll(0.0, 0.5);
            assert_eq!(ctx.scroll_xy().1, 0.5);
            // 0.55 renders at 2 (1.65) as well: no visible change, so none is made.
            ctx.set_scroll(0.0, 0.55);
            assert_eq!(ctx.scroll_xy().1, 0.5);
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
            assert!(
                !ctx.damage.level().needs_geometry(),
                "extending the raster window must not force a re-layout"
            );
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

        /// The grid is a pure function of the layer list and the tile size, and a scroll changes
        /// neither, so an extension must reuse it rather than tile the page again. `generate`
        /// hands out fresh ids from a running counter, so a regeneration would replace every
        /// id - which is what this watches.
        #[test]
        fn extending_the_window_reuses_the_tile_grid() {
            /// Every tile by id, with the layer and page position it was generated for, so a
            /// regenerated grid (new ids) and a moved one (same ids, other geometry) both show.
            fn grid(ctx: &BrowsingContext<DefaultRenderConfig>) -> std::collections::BTreeSet<(String, u64, u64, u64)> {
                let Some(cache) = ctx.pipeline_cache.as_ref() else {
                    unreachable!("pipeline cache must exist");
                };
                let Some(tile_list) = cache.tile_list.as_ref() else {
                    unreachable!("a local render keeps its tile grid");
                };
                tile_list
                    .arena
                    .iter()
                    .map(|(id, tile)| {
                        (
                            id.to_string(),
                            tile.layer_id.as_u64(),
                            tile.rect.x.to_bits(),
                            tile.rect.y.to_bits(),
                        )
                    })
                    .collect()
            }

            let (mut ctx, _calls) = tall_page_context(128);
            ctx.rebuild_pipeline_cache_if_needed();
            let before = grid(&ctx);
            assert!(!before.is_empty(), "a 10 000 px page must produce tiles");

            ctx.set_scroll(0.0, 5000.0);
            assert!(ctx.raster_dirty, "scrolling to unbaked content must raster");
            ctx.rebuild_pipeline_cache_if_needed();

            assert_eq!(before, grid(&ctx), "the extension must not rebuild the tile grid");

            // A reused grid must not accumulate the commands of everything it ever painted:
            // however far the page is scrolled, only what the last pass painted may hold any.
            let mut y = 5000.0;
            while y < 10_000.0 {
                ctx.set_scroll(0.0, y);
                ctx.rebuild_pipeline_cache_if_needed();
                let Some(cache) = ctx.pipeline_cache.as_ref() else {
                    unreachable!("pipeline cache must exist while scrolling");
                };
                let Some(tile_list) = cache.tile_list.as_ref() else {
                    unreachable!("a local render keeps its tile grid");
                };
                let holding = tile_list
                    .arena
                    .values()
                    .filter(|t| t.elements.iter().any(|e| !e.paint_commands.is_empty()))
                    .count();
                let total = tile_list.arena.len();
                assert!(
                    holding * 4 < total,
                    "at scroll {y} the reused grid holds commands for {holding} of {total} tiles,                      so a pass is not releasing what it painted"
                );
                y += VP_H as f64;
            }
            assert_eq!(before, grid(&ctx), "scrolling the page must not rebuild the tile grid");
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

        /// Repro attempt for "the bottom-right tile is white": every grid cell that overlaps the
        /// visible viewport must come back baked, including the partial cells on the right and
        /// bottom edges when the viewport is not a whole number of tiles.
        #[test]
        fn every_tile_cell_covering_the_viewport_is_baked() {
            const VW: u32 = 1000;
            const VH: u32 = 700;

            let config = settings_store::default_config();
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
                width: VW,
                height: VH,
            });

            let html = r#"<html><body style="margin:0"><div style="width:1000px;height:2000px;background:#ddd"></div></body></html>"#;
            let mut doc = gosub_html5::html_compile::<DefaultRenderConfig>(html);
            doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());
            ctx.set_document(Arc::new(doc), None);

            ctx.rebuild_pipeline_cache_if_needed();

            let Some(cache) = ctx.pipeline_cache.as_ref() else {
                unreachable!("pipeline cache must exist after rebuild");
            };
            let present: std::collections::HashSet<(i64, i64)> =
                cache.tiles.iter().map(|t| (t.page_x as i64, t.page_y as i64)).collect();

            let mut missing = Vec::new();
            let mut y = 0i64;
            while y < VH as i64 {
                let mut x = 0i64;
                while x < VW as i64 {
                    if !present.contains(&(x, y)) {
                        missing.push((x, y));
                    }
                    x += 256;
                }
                y += 256;
            }
            assert!(
                missing.is_empty(),
                "tile cells overlapping the viewport were never baked: {missing:?}; baked = {:?}",
                {
                    let mut v: Vec<_> = present.iter().copied().collect();
                    v.sort();
                    v
                }
            );
        }

        /// The startup sequence: a tab is built at one viewport (the engine fallback, or the
        /// host's first guess) and then resized to the real one. Every cell covering the new
        /// viewport must be baked afterwards -- a hole here is a white tile on screen.
        #[test]
        fn viewport_change_keeps_every_visible_tile_baked() {
            fn missing_cells(ctx: &BrowsingContext<DefaultRenderConfig>, vw: u32, vh: u32) -> Vec<(i64, i64)> {
                let cache = ctx.pipeline_cache.as_ref().expect("pipeline cache");
                let present: std::collections::HashSet<(i64, i64)> =
                    cache.tiles.iter().map(|t| (t.page_x as i64, t.page_y as i64)).collect();
                let mut missing = Vec::new();
                let mut y = 0i64;
                while y < vh as i64 {
                    let mut x = 0i64;
                    while x < vw as i64 {
                        if !present.contains(&(x, y)) {
                            missing.push((x, y));
                        }
                        x += 256;
                    }
                    y += 256;
                }
                missing
            }

            let config = settings_store::default_config();
            let mut ctx: BrowsingContext<DefaultRenderConfig> = BrowsingContext::new(config);
            let calls = Arc::new(AtomicUsize::new(0));
            ctx.set_rasterizer(
                Box::new(SolidRasterizer {
                    calls: Arc::clone(&calls),
                }),
                RasterStrategy::ParallelCached,
            );

            // Built at the fallback size first, exactly like a tab created before its host
            // window is allocated.
            ctx.set_viewport(Viewport {
                x: 0,
                y: 0,
                width: 1280,
                height: 800,
            });
            let html = r#"<html><body style="margin:0"><div style="width:100%;height:2000px;background:#ddd"></div></body></html>"#;
            let mut doc = gosub_html5::html_compile::<DefaultRenderConfig>(html);
            doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());
            ctx.set_document(Arc::new(doc), None);
            ctx.rebuild_pipeline_cache_if_needed();
            assert!(
                missing_cells(&ctx, 1280, 800).is_empty(),
                "holes already at the fallback size: {:?}",
                missing_cells(&ctx, 1280, 800)
            );

            // Now the real GLArea size lands.
            ctx.set_viewport(Viewport {
                x: 0,
                y: 0,
                width: 1000,
                height: 700,
            });
            ctx.rebuild_pipeline_cache_if_needed();

            let missing = missing_cells(&ctx, 1000, 700);
            assert!(missing.is_empty(), "white tiles after the viewport change: {missing:?}");
        }

        /// A hover repaint reuses the cached layout and carries unaffected tiles forward. If the
        /// carry-over drops one, that tile turns white on screen and stays white until another
        /// repaint happens to cover it -- which is exactly "the bottom-right tile is white until
        /// I move the mouse".
        #[test]
        fn hover_repaint_does_not_drop_visible_tiles() {
            const VW: u32 = 1000;
            const VH: u32 = 700;

            fn cells(ctx: &BrowsingContext<DefaultRenderConfig>) -> std::collections::BTreeSet<(i64, i64)> {
                ctx.pipeline_cache
                    .as_ref()
                    .expect("pipeline cache")
                    .tiles
                    .iter()
                    .map(|t| (t.page_x as i64, t.page_y as i64))
                    .collect()
            }

            let config = settings_store::default_config();
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
                width: VW,
                height: VH,
            });

            let html = r#"<html><body style="margin:0">
                <a href="https://example.com" style="display:block;width:300px;height:100px">hover me</a>
                <div style="width:1000px;height:2000px;background:#ddd"></div>
                </body></html>"#;
            let mut doc = gosub_html5::html_compile::<DefaultRenderConfig>(html);
            doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());
            ctx.set_document(Arc::new(doc), None);
            ctx.rebuild_pipeline_cache_if_needed();

            let before = cells(&ctx);
            assert!(!before.is_empty(), "nothing baked on the first pass");

            // Hover the link in the top-left, then repaint. Tiles far from the pointer must be
            // carried forward untouched, not dropped.
            let _ = ctx.update_hover(50.0, 50.0);
            ctx.rebuild_pipeline_cache_if_needed();
            let after = cells(&ctx);

            let lost: Vec<_> = before.difference(&after).copied().collect();
            assert!(
                lost.is_empty(),
                "hover repaint dropped tiles that were baked before: {lost:?}"
            );
        }

        /// Regression: a viewport narrower than the laid-out page must still paint every tile
        /// COLUMN, not just the one at x = 0.
        ///
        /// A zero width makes the layouter fall back to `MAX_CONTENT`, so the page lays out far
        /// wider than the viewport. The painter's page rect used to take its width from the
        /// viewport, which collapsed it to a degenerate zero-width envelope; the r-tree query
        /// then matched only tiles whose left edge touches x = 0, so exactly one 256 px column
        /// was ever painted and rasterized. A host that creates a tab before its window is
        /// allocated (GTK reports 0x0 for an unallocated widget) hit this on every tab switch.
        #[test]
        fn narrow_viewport_still_paints_every_tile_column() {
            let config = settings_store::default_config();
            let mut ctx: BrowsingContext<DefaultRenderConfig> = BrowsingContext::new(config);
            let calls = Arc::new(AtomicUsize::new(0));
            ctx.set_rasterizer(
                Box::new(SolidRasterizer {
                    calls: Arc::clone(&calls),
                }),
                RasterStrategy::ParallelCached,
            );
            // Width 0 is the shape an unallocated host window reports; height is kept non-zero
            // so the raster window still admits the top rows and the test isolates the width.
            ctx.set_viewport(Viewport {
                x: 0,
                y: 0,
                width: 0,
                height: VP_H,
            });

            let html = r#"<html><body style="margin:0"><div style="width:2000px;height:300px;background:#ddd"></div></body></html>"#;
            let mut doc = gosub_html5::html_compile::<DefaultRenderConfig>(html);
            doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());
            ctx.set_document(Arc::new(doc), None);

            ctx.rebuild_pipeline_cache_if_needed();

            let Some(cache) = ctx.pipeline_cache.as_ref() else {
                unreachable!("pipeline cache must exist after rebuild");
            };
            assert!(
                cache.page_height > 0.0,
                "page must lay out with a real height, got {}",
                cache.page_height
            );
            let columns: std::collections::BTreeSet<i64> = cache.tiles.iter().map(|t| t.page_x as i64).collect();
            assert!(
                columns.iter().any(|&x| x > 0),
                "only the x=0 column was rasterized ({columns:?}); the page rect collapsed to zero width"
            );
            assert!(
                columns.len() >= 2,
                "expected several tile columns across a 2000px page, got {columns:?}"
            );
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
