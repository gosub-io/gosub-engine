//! Browsing context and tab runtime state.
//!
//! [`BrowsingContext`] is the per-tab state owned by [`TabWorker`]. It holds the document,
//! viewport, storage handles, and the render list that the active [`RenderBackend`] will paint.
//!
//! When the `pipeline` Cargo feature is enabled the full rendering pipeline
//! (RenderTree → Taffy layout → Layers → Tiles → PaintCommands) is used to populate the
//! render list instead of the placeholder text-dump.

use crate::engine::storage::{StorageArea, StorageHandles};
use crate::render::backend::RenderContext;
use crate::render::{Color, DisplayItem, RenderList, Viewport};
use gosub_interface::config::HasDocument;
use gosub_interface::document::Document;
use std::marker::PhantomData;
use std::sync::Arc;
use url::Url;

/// BrowsingContext dedicated to a specific tab
pub struct BrowsingContext<C: HasDocument> {
    /// Current URL being processed
    current_url: Option<Url>,
    /// "DOM" Document
    document: Option<Arc<C::Document>>,
    /// True when the tab has failed loading (mostly net issues)
    failed: bool,
    /// Storage handles for local and session storage
    storage: Option<StorageHandles>,

    // Rendering commands to paint the tab onto a surface
    render_list: RenderList,
    /// Render dirty flag — true when the render list must be rebuilt
    render_dirty: bool,
    /// Viewport for the tab
    viewport: Viewport,
    /// Epoch of the scene, incremented on every rebuild
    scene_epoch: u64,

    /// DOM dirty flag
    dom_dirty: bool,
    /// Style dirty flag
    style_dirty: bool,
    /// Layout dirty flag
    layout_dirty: bool,

    _phantom: PhantomData<C>,
}

impl<C: HasDocument> Default for BrowsingContext<C> {
    fn default() -> Self {
        Self::new()
    }
}

impl<C: HasDocument> BrowsingContext<C> {
    /// Creates a new runtime browsing context.
    pub fn new() -> BrowsingContext<C> {
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
            _phantom: PhantomData,
        }
    }

    pub fn bind_storage(&mut self, local: Arc<dyn StorageArea>, session: Arc<dyn StorageArea>) {
        self.storage = Some(StorageHandles { local, session });
    }
    pub fn local_storage(&self) -> Option<Arc<dyn StorageArea>> {
        self.storage.as_ref().map(|s| s.local.clone())
    }
    pub fn session_storage(&self) -> Option<Arc<dyn StorageArea>> {
        self.storage.as_ref().map(|s| s.session.clone())
    }

    /// Sets the document for the given tab
    pub fn set_document(&mut self, doc: Arc<C::Document>) {
        self.document = Some(doc);
        self.dom_dirty = true;
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

    /// Returns the render list
    #[inline]
    pub fn render_list(&self) -> &RenderList {
        &self.render_list
    }

    /// Returns true when the loading failed
    pub fn has_failed(&self) -> bool {
        self.failed
    }

    /// Returns the current loaded URL (or None when nothing has loaded yet)
    pub fn current_url(&self) -> Option<&Url> {
        self.current_url.as_ref()
    }
}

// ─── Pipeline-backed rebuild ─────────────────────────────────────────────────
// When the `pipeline` feature is enabled AND the BrowsingContext has a document,
// we run the full pipeline to produce paint commands and convert them to DisplayItems.
// When the feature is disabled, or when there is no document yet, we fall back
// to a simple text-dump placeholder.

impl<C: HasDocument + Send + Sync + 'static> BrowsingContext<C>
where
    C::Document: gosub_interface::document::Document<C> + Send + Sync,
    <<C::Document as gosub_interface::document::Document<C>>::Node as gosub_interface::node::Node<C>>::ElementData:
        gosub_interface::node::ElementDataType<C>,
    <<C::Document as gosub_interface::document::Document<C>>::Node as gosub_interface::node::Node<C>>::TextData:
        gosub_interface::node::TextDataType,
    <<C::Document as gosub_interface::document::Document<C>>::Node as gosub_interface::node::Node<C>>::CommentData:
        gosub_interface::node::CommentDataType,
{
    /// Build/refresh the render list if needed.
    pub fn rebuild_render_list_if_needed(&mut self) {
        if !self.render_dirty {
            return;
        }

        #[cfg(feature = "pipeline")]
        {
            if let Some(doc) = self.document.as_ref() {
                if self.run_pipeline(doc.clone()) {
                    self.render_dirty = false;
                    self.scene_epoch = self.scene_epoch.wrapping_add(1);
                    self.dom_dirty = false;
                    self.style_dirty = false;
                    self.layout_dirty = false;
                    return;
                }
            }
        }

        // Fallback: render the document as plain text lines
        self.rebuild_render_list_fallback();
    }

    /// Fallback: dumps the document as text lines into the render list.
    fn rebuild_render_list_fallback(&mut self) {
        let mut rl = RenderList::default();
        rl.items.push(DisplayItem::Clear {
            color: Color::new(0.95, 0.95, 0.95, 1.0),
        });

        let c = Color::new(0.0, 0.0, 0.0, 1.0);
        let font_size = 14.0f32;
        let mut y = 0.0f32;

        if let Some(doc) = self.document.as_ref() {
            for line in doc.write().lines() {
                rl.items.push(DisplayItem::TextRun {
                    x: 0.0,
                    y,
                    text: line.to_string(),
                    size: font_size,
                    color: c,
                    max_width: Some(self.viewport.width as f32),
                });
                y += font_size;
            }
        }

        self.render_list = rl;
        self.render_dirty = false;
        self.scene_epoch = self.scene_epoch.wrapping_add(1);
        self.dom_dirty = false;
        self.style_dirty = false;
        self.layout_dirty = false;
    }

    /// Run the full rendering pipeline and populate `self.render_list`.
    /// Returns `true` on success.
    #[cfg(feature = "pipeline")]
    fn run_pipeline(&mut self, gosub_doc: Arc<C::Document>) -> bool {
        use gosub_pipeline::bridge::build_pipeline_document;
        use gosub_pipeline::common::geo::Dimension;
        use gosub_pipeline::layering::layer::LayerList;
        use gosub_pipeline::layouter::{taffy::TaffyLayouter, CanLayout};
        use gosub_pipeline::painter::{PaintOptions, Painter};
        use gosub_pipeline::rendertree_builder::tree::RenderTree;
        use gosub_pipeline::tiler::TileList;

        // Get base URL from document
        let base_url = gosub_doc
            .url()
            .map(|u| u.to_string())
            .unwrap_or_else(|| "about:blank".to_string());

        // Step 1 — bridge the gosub document to a pipeline document
        let pipeline_doc = build_pipeline_document::<C>(&*gosub_doc, &base_url);

        // Step 2 — build the render tree (filters invisible elements)
        let mut render_tree = RenderTree::new(pipeline_doc);
        render_tree.parse();

        // Step 3 — layout (Taffy); returns owned LayoutTree
        let viewport_dim = if self.viewport.width > 0 && self.viewport.height > 0 {
            Some(Dimension::new(self.viewport.width as f64, self.viewport.height as f64))
        } else {
            None
        };
        let mut layouter = TaffyLayouter::new();
        let layout_tree = layouter.layout(render_tree, viewport_dim, 1.0);

        // Collect debug outlines before layout_tree is consumed
        let debug_outlines: Vec<DisplayItem> = {
            const PALETTE: [(f32, f32, f32); 6] = [
                (1.0, 0.0, 0.0), // red
                (0.0, 0.7, 0.0), // green
                (0.0, 0.0, 1.0), // blue
                (1.0, 0.5, 0.0), // orange
                (0.7, 0.0, 0.7), // purple
                (0.0, 0.7, 0.7), // teal
            ];
            layout_tree
                .arena
                .values()
                .enumerate()
                .map(|(i, node)| {
                    let (r, g, b) = PALETTE[i % PALETTE.len()];
                    let bb = node.box_model.border_box;
                    DisplayItem::Outline {
                        x: bb.x as f32,
                        y: bb.y as f32,
                        w: bb.width as f32,
                        h: bb.height as f32,
                        color: Color::new(r, g, b, 0.6),
                    }
                })
                .collect()
        };

        // Step 4 — layering; LayerList::new takes owned LayoutTree, wraps it in Arc internally
        // and already calls generate_layers() internally
        let layer_list = LayerList::new(layout_tree);

        // Step 5 — tiling; TileList::new takes owned LayerList, wraps it in Arc internally
        let tile_dim = Dimension::new(512.0, 512.0);
        let mut tile_list = TileList::new(layer_list, tile_dim);
        tile_list.generate();

        // Step 6 — paint: collect all paint commands from all tiles
        // Painter takes Arc<LayerList>, which is stored inside tile_list
        let painter = Painter::new(tile_list.layer_list.clone());
        let paint_opts = PaintOptions::default();

        // Build a white background render list, then append paint commands as DisplayItems
        let mut rl = RenderList::default();
        rl.items.push(DisplayItem::Clear {
            color: Color::new(1.0, 1.0, 1.0, 1.0),
        });

        let vp_rect = gosub_pipeline::common::geo::Rect::new(
            self.viewport.x as f64,
            self.viewport.y as f64,
            self.viewport.width as f64,
            self.viewport.height as f64,
        );

        // Iterate visible layers in order and paint each tile that intersects the viewport
        let layer_ids: Vec<_> = {
            let guard = tile_list.layer_list.layer_ids.read().unwrap();
            guard.clone()
        };

        for layer_id in layer_ids {
            // get_intersecting_tiles takes Rect by value
            let visible_tiles = tile_list.get_intersecting_tiles(layer_id, vp_rect);
            for tile_id in visible_tiles {
                let Some(tile) = tile_list.arena.get(&tile_id) else {
                    continue;
                };
                for tiled_element in &tile.elements {
                    let commands = painter.paint(tiled_element, &paint_opts);
                    for cmd in commands {
                        if let Some(item) = paint_command_to_display_item(cmd) {
                            rl.items.push(item);
                        }
                    }
                }
            }
        }

        rl.items.extend(debug_outlines);

        self.render_list = rl;
        true
    }
}

/// Convert a gosub_pipeline `PaintCommand` to a gosub_engine_core `DisplayItem`.
#[cfg(feature = "pipeline")]
fn paint_command_to_display_item(cmd: gosub_pipeline::painter::commands::PaintCommand) -> Option<DisplayItem> {
    use gosub_pipeline::painter::commands::brush::Brush;
    use gosub_pipeline::painter::commands::PaintCommand;

    match cmd {
        PaintCommand::Text(t) => {
            // Extract colour from brush (solid only; images default to black)
            let color = match &t.brush {
                Brush::Solid(c) => Color::new(c.r(), c.g(), c.b(), c.a()),
                _ => Color::new(0.0, 0.0, 0.0, 1.0),
            };
            Some(DisplayItem::TextRun {
                x: t.rect.x as f32,
                y: (t.rect.y + t.rect.height) as f32, // baseline at bottom of box
                text: t.text,
                size: t.font_info.size as f32,
                color,
                max_width: Some(t.rect.width as f32),
            })
        }
        PaintCommand::Rectangle(r) => {
            let rect = r.rect();
            let color = match r.background() {
                Some(Brush::Solid(c)) => Color::new(c.r(), c.g(), c.b(), c.a()),
                _ => Color::new(0.0, 0.0, 0.0, 0.0), // transparent
            };
            // Skip fully-transparent rectangles to reduce noise
            if color.a == 0.0 {
                return None;
            }
            Some(DisplayItem::Rect {
                x: rect.x as f32,
                y: rect.y as f32,
                w: rect.width as f32,
                h: rect.height as f32,
                color,
            })
        }
        PaintCommand::Svg(_) => None, // SVG not yet wired to DisplayItem
    }
}

impl<C: HasDocument> RenderContext for BrowsingContext<C> {
    fn viewport(&self) -> &Viewport {
        &self.viewport
    }

    fn render_list(&self) -> &RenderList {
        &self.render_list
    }
}
