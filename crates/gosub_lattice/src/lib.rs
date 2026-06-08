pub mod compute;
pub mod grid;
pub mod mock;
pub mod model;
pub mod sizing;
pub mod types;
mod tests;

pub use compute::compute_table_layout;
pub use types::{
    BorderCollapse, BoxEdges, CellLayout, CssProp, CssLength, TableRole, TableSizing,
};

use std::fmt::Debug;
use std::hash::Hash;

/// Adapter trait that `gosub_lattice` uses to read from and write to an external layout tree.
///
/// The implementor (e.g. `gosub_taffy`'s `LayoutDocument`) translates between the engine's
/// internal representations and the flat types expected here.
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
    /// is correct — explicit CSS `height` on the cell will still be respected
    /// by the row-height algorithm.
    fn layout_cell(&mut self, id: Self::NodeId, available_width: f32) -> f32;

    /// Returns the natural (pre-pass) border-box width of cell `id` as
    /// measured by the layout engine in a prior pass (e.g. Taffy).  Used to
    /// distribute auto column widths proportionally to content width rather
    /// than equally.  Return `0.0` for mock/test trees.
    fn cell_content_width(&self, id: Self::NodeId) -> f32 {
        0.0
    }
}
