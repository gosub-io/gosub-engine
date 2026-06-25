use crate::common::geo::Rect;
use crate::layouter::LayoutElementId;
use crate::tiler::TileList;
use parking_lot::RwLock;
use std::fmt::Debug;

#[derive(Debug)]
pub enum WireframeState {
    None,
    Only,
    Both,
}

/// Per-tab render settings passed through the pipeline instead of living in a global.
pub struct BrowserState {
    /// List of layers that will be visible are set to true
    pub visible_layer_list: Vec<bool>,
    /// Defines if we need to draw wireframes, or the actual content, or both
    pub wireframed: WireframeState,
    /// Just show the hovered debug node in wireframe
    pub debug_hover: bool,
    /// Show the tile grid
    pub show_tilegrid: bool,
    /// Draw a 1px red border around every table-cell element (set via GOSUB_DEBUG_TABLE_CELLS=1)
    pub debug_table_cells: bool,
    /// When set, this is the element that is currently hovered upon
    pub current_hovered_element: Option<LayoutElementId>,
    /// Current viewport offset + size
    pub viewport: Rect,
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
            .field("debug_table_cells", &self.debug_table_cells)
            .field("current_hovered_element", &self.current_hovered_element)
            .field("viewport", &self.viewport)
            .field("dpi_scale_factor", &self.dpi_scale_factor)
            .finish()
    }
}
