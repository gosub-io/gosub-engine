use std::fmt::Debug;
use std::sync::{Arc, OnceLock, RwLock};
use crate::common::document::document::Document;
use crate::common::geo::Rect;
use crate::layouter::LayoutElementId;
use crate::tiler::TileList;

#[derive(Debug)]
pub enum WireframeState {
    None,
    Only,
    Both,
}

/// Things that can change in the browser is stored in this structure. It keeps the current rendering pipeline (in the form of a layer_list),
/// and some things that we can control, or is controlled by the user (like current_hovered_element).
pub struct BrowserState {
    /// List of layers that will be visible are set to true
    pub visible_layer_list: Vec<bool>,
    /// Defines if we need to draw wireframes, or the actual content, or both
    pub wireframed: WireframeState,
    /// Just show the hovered debug node in wireframe
    pub debug_hover: bool,
    /// Show the tile grid
    pub show_tilegrid: bool,
    /// When set, this is the element that is currently hovered upon
    pub current_hovered_element: Option<LayoutElementId>,
    /// Current viewport offset + size
    pub viewport: Rect,
    /// Main document that is currently being rendered
    pub document: Arc<Document>,
    /// LayerList that is currently being rendered
    pub tile_list: Option<RwLock<TileList>>,
    /// Scale factor for DPI
    pub dpi_scale_factor: f32,
}

impl Debug for BrowserState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserState")
            .field("visible_layer_list", &self.visible_layer_list)
            .field("wireframed", &self.wireframed)
            .field("debug_hover", &self.debug_hover)
            .field("show_tilegrid", &self.show_tilegrid)
            .field("current_hovered_element", &self.current_hovered_element)
            .field("viewport", &self.viewport)
            .field("dpi_scale_factor", &self.dpi_scale_factor)
            .finish()
    }
}


static BROWSER_STATE: OnceLock<Arc<RwLock<BrowserState>>> = OnceLock::new();

pub fn init_browser_state(state: BrowserState) {
    BROWSER_STATE.set(Arc::new(RwLock::new(state))).expect("Failed to set browser state");
}

pub fn get_browser_state() -> Arc<RwLock<BrowserState>> {
    BROWSER_STATE.get().expect("Failed to get browser state").clone()
}