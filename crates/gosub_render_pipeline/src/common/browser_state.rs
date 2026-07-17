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
    /// Indexed by layer id; `true` means the layer is drawn.
    pub visible_layer_list: Vec<bool>,
    /// Whether to draw wireframes, the actual content, or both.
    pub wireframed: WireframeState,
    /// Restrict wireframing to the hovered node only.
    pub debug_hover: bool,
    pub show_tilegrid: bool,
    /// Draw a 1px red border around every table-cell element (set via GOSUB_DEBUG_TABLE_CELLS=1)
    pub debug_table_cells: bool,
    pub current_hovered_element: Option<LayoutElementId>,
    /// Current viewport offset + size
    pub viewport: Rect,
    pub tile_list: Option<RwLock<TileList>>,
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
