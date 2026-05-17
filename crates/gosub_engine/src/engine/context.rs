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
    use gosub_pipeline::common::document::pipeline_doc::GosubDocumentAdapter;
    use gosub_pipeline::common::document::style::{StyleProperty, StyleValue};
    use gosub_pipeline::common::geo::Dimension as PipelineDimension;
    use gosub_pipeline::layouter::taffy::TaffyLayouter;
    use gosub_pipeline::layouter::{CanLayout, ElementContext};
    use gosub_pipeline::rendertree_builder::RenderTree;

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

    let root_id = layout_tree.root_id;
    let mut stack = vec![root_id];
    while let Some(id) = stack.pop() {
        let Some(el) = layout_tree.get_node_by_id(id) else {
            continue;
        };

        match &el.context {
            ElementContext::None => {
                let bg = layout_tree
                    .render_tree
                    .doc
                    .get_style(el.dom_node_id, StyleProperty::BackgroundColor);
                if let Some(StyleValue::Color(c)) = bg {
                    let bb = &el.box_model.border_box;
                    if bb.width > 0.0 && bb.height > 0.0 {
                        rl.items.push(DisplayItem::Rect {
                            x: bb.x as f32,
                            y: bb.y as f32,
                            w: bb.width as f32,
                            h: bb.height as f32,
                            color: pipeline_color_to_engine(&c),
                        });
                    }
                }
            }
            ElementContext::Text(ctx) => {
                if !ctx.text.trim().is_empty() {
                    let bb = &el.box_model.content_box;
                    rl.items.push(DisplayItem::TextRun {
                        x: bb.x as f32,
                        y: (bb.y + ctx.text_offset.y) as f32,
                        text: ctx.text.clone(),
                        size: ctx.font_info.size as f32,
                        color: Color::new(0.0, 0.0, 0.0, 1.0),
                        max_width: if bb.width > 0.0 { Some(bb.width as f32) } else { None },
                    });
                }
            }
            ElementContext::Image(_) | ElementContext::Svg(_) => {}
        }

        for &child_id in el.children.iter().rev() {
            stack.push(child_id);
        }
    }
}

#[cfg(feature = "pipeline")]
fn pipeline_color_to_engine(c: &gosub_pipeline::common::document::style::Color) -> Color {
    use gosub_pipeline::common::document::style::Color as SC;
    match c {
        SC::Rgb(r, g, b) => Color::from_u8(*r, *g, *b, 255),
        SC::Rgba(r, g, b, a) => Color::from_u8(*r, *g, *b, (*a * 255.0) as u8),
        SC::Named(name) => match name.as_str() {
            "white" => Color::new(1.0, 1.0, 1.0, 1.0),
            "black" => Color::new(0.0, 0.0, 0.0, 1.0),
            "red" => Color::new(1.0, 0.0, 0.0, 1.0),
            "green" => Color::new(0.0, 0.5, 0.0, 1.0),
            "blue" => Color::new(0.0, 0.0, 1.0, 1.0),
            "transparent" => Color::new(0.0, 0.0, 0.0, 0.0),
            _ => Color::new(0.0, 0.0, 0.0, 0.0),
        },
    }
}
