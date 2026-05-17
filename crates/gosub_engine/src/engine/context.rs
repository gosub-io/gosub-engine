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
use crate::render::{Color, DisplayItem, RenderList, Viewport};
use std::sync::Arc;
use url::Url;

#[cfg(feature = "pipeline")]
use crate::html::HtmlEngineConfig;
// #[derive(Debug, thiserror::Error)]
// pub enum LoadError {
//     #[error("navigation cancelled")]
//     Cancelled,
//     #[error(transparent)]
//     Net(#[from] reqwest::Error),
// }

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
    /// Viewport for the tab, used to determine what part of the page to render
    viewport: Viewport,
    /// Epoch of the scene, used to determine if the scene has changed
    scene_epoch: u64,

    /// DOM dirty flag, used to determine if the DOM has changed
    dom_dirty: bool,
    /// Style dirty flag, used to determine if the styles have changed
    style_dirty: bool,
    /// Layout dirty flag, used to determine if the layout has changed
    layout_dirty: bool,
}

impl BrowsingContext {
    /// Creates a new runtime browsing context.
    pub(crate) fn new() -> BrowsingContext {
        Self {
            current_url: None,
            document: None,
            failed: false,
            storage: None, // Default no storage unless binding manually by a tab
            render_list: RenderList::new(),
            render_dirty: false,
            viewport: Viewport::default(),
            scene_epoch: 0,
            dom_dirty: false,
            style_dirty: false,
            layout_dirty: false,
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
        self.dom_dirty = true; // Mark the DOM as dirty, so it will be rendered
        self.style_dirty = true;
        self.layout_dirty = true;
        self.invalidate_render();
    }

    pub fn set_viewport(&mut self, vp: Viewport) {
        if self.viewport != vp {
            self.viewport = vp;
            self.layout_dirty = true;
            self.invalidate_render();
        }
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
    pub fn rebuild_render_list_if_needed(&mut self) {
        if !self.render_dirty {
            return;
        }

        let mut rl = RenderList::default();

        #[cfg(feature = "pipeline")]
        if let Some(doc) = &self.document {
            pipeline_render(doc.clone(), &self.viewport, &mut rl);
        }

        #[cfg(not(feature = "pipeline"))]
        rl.items.push(DisplayItem::Clear {
            color: Color::new(0.6, 0.6, 0.6, 1.0),
        });

        self.render_list = rl;
        self.render_dirty = false;
        self.scene_epoch = self.scene_epoch.wrapping_add(1);

        self.dom_dirty = false;
        self.style_dirty = false;
        self.layout_dirty = false;
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

/// Runs the full gosub_pipeline (rendertree → layout) and fills `rl` with
/// backend-agnostic [`DisplayItem`]s derived from the computed layout.
///
/// The function is a no-op when the `pipeline` Cargo feature is not enabled.
#[cfg(feature = "pipeline")]
fn pipeline_render(doc: Arc<EngineDocument>, viewport: &Viewport, rl: &mut RenderList) {
    use gosub_pipeline::common::browser_state::{BrowserState, WireframeState};
    use gosub_pipeline::common::document::pipeline_doc::GosubDocumentAdapter;
    use gosub_pipeline::common::geo::{Dimension as PipelineDimension, Rect as PipelineRect};
    use gosub_pipeline::layering::layer::LayerList;
    use gosub_pipeline::layouter::taffy::TaffyLayouter;
    use gosub_pipeline::layouter::CanLayout;
    use gosub_pipeline::painter::Painter;
    use gosub_pipeline::rendertree_builder::RenderTree;
    use gosub_pipeline::tiler::{TileList, TileState};

    rl.items.push(DisplayItem::Clear {
        color: Color::new(1.0, 1.0, 1.0, 1.0),
    });

    let adapter = GosubDocumentAdapter::<HtmlEngineConfig>::new(doc);
    let mut render_tree = RenderTree::new(Arc::new(adapter));
    render_tree.parse();

    let vp_dim = if viewport.width > 0 && viewport.height > 0 {
        Some(PipelineDimension::new(viewport.width as f64, viewport.height as f64))
    } else {
        None
    };

    let mut layouter = TaffyLayouter::new();
    let layout_tree = layouter.layout(render_tree, vp_dim, 1.0);
    let layer_list = LayerList::new(layout_tree);

    // Stage 4: tiling
    let mut tile_list = TileList::new(layer_list, PipelineDimension::new(256.0, 256.0));
    tile_list.generate();

    // Stage 5: painting — populate paint_commands on each dirty tile element
    let vp_rect = PipelineRect::new(0.0, 0.0, viewport.width as f64, viewport.height as f64);
    let layer_ids = tile_list.layer_list.layer_ids.read().clone();
    let paint_state = BrowserState {
        visible_layer_list: vec![true; layer_ids.len()],
        wireframed: WireframeState::None,
        debug_hover: false,
        current_hovered_element: None,
        show_tilegrid: false,
        viewport: vp_rect,
        tile_list: None,
        dpi_scale_factor: 1.0,
    };
    let painter = Painter::new(tile_list.layer_list.clone());
    for &layer_id in &layer_ids {
        let tile_ids = tile_list.get_intersecting_tiles(layer_id, vp_rect);
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

    // Stage 6: rasterize — convert paint_commands into pixel textures
    #[cfg(feature = "backend_cairo")]
    let texture_store = {
        use gosub_pipeline::common::media::MediaStore;
        use gosub_pipeline::common::texture_store::TextureStore;
        use gosub_pipeline::rasterizer::cairo::CairoRasterizer;
        use gosub_pipeline::rasterizer::Rasterable;

        let media_store = MediaStore::new();
        let mut texture_store = TextureStore::new();
        let rasterizer = CairoRasterizer::new();
        for &layer_id in &layer_ids {
            let tile_ids = tile_list.get_intersecting_tiles(layer_id, vp_rect);
            for tile_id in tile_ids {
                if let Some(tile) = tile_list.get_tile_mut(tile_id) {
                    if tile.state == TileState::Dirty {
                        match rasterizer.rasterize(tile, &mut texture_store, &media_store) {
                            Some(texture_id) => {
                                tile.texture_id = Some(texture_id);
                                tile.state = TileState::Clean;
                            }
                            None => {
                                tile.state = TileState::Empty;
                            }
                        }
                    }
                }
            }
        }
        texture_store
    };

    // Stage 7: compositing — emit one Blit item per clean tile for the Cairo backend
    #[cfg(feature = "backend_cairo")]
    for tile in tile_list.arena.values() {
        if let (Some(texture_id), true) = (tile.texture_id, tile.state == TileState::Clean) {
            if let Some(tex) = texture_store.get(texture_id) {
                rl.items.push(DisplayItem::Blit {
                    x: tile.rect.x as f32,
                    y: tile.rect.y as f32,
                    w: tex.width as u32,
                    h: tex.height as u32,
                    data: tex.data.clone(),
                });
            }
        }
    }

}
