pub mod compute;
pub mod geo;
pub mod grid;
pub mod mock;
pub mod model;
pub mod sizing;
mod tests;
pub mod types;

pub use compute::compute_table_layout;
pub use types::{BorderCollapse, BoxEdges, CellLayout, CssLength, CssProp, TableRole, TableSizing, VerticalAlign};

use std::fmt::Debug;
use std::hash::Hash;

/// Adapter trait that `gosub_lattice` uses to read from and write to an external layout tree.
///
/// The implementor (e.g. `gosub_render_pipeline`'s `PipelineTableTree`) translates between the
/// engine's internal representations and the flat types expected here.
pub trait TableTree {
    type NodeId: Copy + Clone + Eq + Hash + Debug;

    /// Returns the children of `id` in document order.
    fn children(&self, id: Self::NodeId) -> Vec<Self::NodeId>;

    /// CSS table display role of `id`.
    fn table_role(&self, id: Self::NodeId) -> TableRole;

    /// CSS length value for a given property on `id`.
    fn css_length(&self, id: Self::NodeId, prop: CssProp) -> CssLength;

    /// Returns an HTML attribute parsed as `usize` (used for `colspan`, `rowspan`).
    fn attr_usize(&self, id: Self::NodeId, attr: &str) -> Option<usize>;

    /// Writes the computed layout for `id` back to the tree.
    fn set_layout(&mut self, id: Self::NodeId, layout: CellLayout);

    /// Lay out the children of the cell `id` given its available inner content
    /// width (border-box width minus the cell's own border and padding).
    ///
    /// The implementor should run the normal layout engine on the cell's
    /// subtree (e.g. block/flex layout via Taffy) and return the actual
    /// content height the children occupy.
    ///
    /// For mock/test trees that carry no real child content, returning `0.0`
    /// is correct - explicit CSS `height` on the cell will still be respected
    /// by the row-height algorithm.
    fn layout_cell(&mut self, id: Self::NodeId, available_width: f32) -> f32;

    /// Returns the natural (pre-pass) border-box width of cell `id` as
    /// measured by the layout engine in a prior pass (e.g. Taffy).  Used to
    /// distribute auto column widths proportionally to content width rather
    /// than equally.  Return `0.0` for mock/test trees.
    /// Intrinsic (content-driven) border-box widths of cell `id` as measured
    /// by the layout engine: `(min_content, max_content)`. Min-content is the
    /// narrowest the cell can get without overflowing (longest word, widest
    /// replaced box); max-content is its width with no wrapping at all. Takes
    /// `&mut self` because implementors typically have to run layout passes to
    /// measure. Return `(0.0, 0.0)` for mock/test trees with no real content.
    fn cell_intrinsic_widths(&mut self, _id: Self::NodeId) -> (f32, f32) {
        (0.0, 0.0)
    }

    /// Whether `cell_intrinsic_widths` returns REAL measurements. When true, an
    /// all-zero result means the content genuinely has no width (empty cells) and the
    /// table shrinks to fit per CSS 2 §17.5.2.2; when false (mock/test trees using the
    /// default stub), zero means "no information" and auto tables fill the available
    /// width instead of collapsing.
    fn measures_intrinsics(&self) -> bool {
        false
    }

    /// Distance from the top of cell `id`'s BORDER box to the baseline of its first
    /// in-flow line box, measured after `layout_cell` has run for the cell. `None`
    /// when the cell has no line box (empty cells align top).
    fn cell_baseline(&mut self, _id: Self::NodeId) -> Option<f32> {
        None
    }

    /// `vertical-align` for cell `id`, resolved through the cascade (including
    /// the HTML rendering-spec pattern of `inherit` on cells picking up
    /// `middle` from the row/section) by the implementor.
    fn vertical_align(&self, _id: Self::NodeId) -> types::VerticalAlign {
        types::VerticalAlign::Top
    }

    /// Under `border-collapse`, gives the implementor cell `id`'s LAYOUT
    /// border - half the resolved boundary width per edge, since collapsed
    /// borders are centered on the grid lines - BEFORE any measurement
    /// happens. Implementors backed by a layout engine should override the
    /// cell's border widths in the engine's style so content sits where the
    /// collapse geometry says.
    ///
    /// `edge_owners` (`[top, right, bottom, left]`) names, per edge, the cell
    /// whose CSS border style/color must be used when painting this cell's
    /// half of the boundary: `None` means the cell's own border (it won or the
    /// edge is on the table perimeter), `Some(other)` means it lost the
    /// conflict and paints its half in the winner's style. Painting each half
    /// from its own cell keeps the result independent of cell paint order.
    fn set_collapsed_cell_borders(
        &mut self,
        _id: Self::NodeId,
        _layout: types::BoxEdges,
        _edge_owners: [Option<Self::NodeId>; 4],
    ) {
    }
}
