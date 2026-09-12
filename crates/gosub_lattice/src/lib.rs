pub mod compute;
pub mod geo;
pub mod grid;
pub mod mock;
pub mod model;
pub mod sizing;
mod tests;
pub mod types;

pub use compute::compute_table_layout;
pub use types::{BorderCollapse, BoxEdges, CellLayout, CssLength, CssProp, TableRole, TableSizing};

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

    /// Whether this table box sizes to its contents rather than filling the space offered.
    ///
    /// CSS auto table width is `max(MIN, min(MAX, available))`; a table that is floated or
    /// `inline-table` is always shrink-to-fit. Implementors that do not model floats can leave
    /// this `false`, which keeps the "fill the available width" behaviour.
    fn table_shrink_to_fit(&self, _id: Self::NodeId) -> bool {
        false
    }

    /// Whether the caption goes below the table rather than above (`caption-side: bottom`).
    fn caption_at_bottom(&self, _id: Self::NodeId) -> bool {
        false
    }

    /// Height of the table's caption when laid out at `width`.
    ///
    /// A caption does not take part in the column algorithm - it is placed across the finished
    /// table (CSS 2.1 §17.4) - so it is measured separately, after the width is known.
    fn caption_height(&mut self, _id: Self::NodeId, _width: f32) -> f32 {
        0.0
    }

    /// Returns the natural (pre-pass) border-box width of cell `id` as
    /// measured by the layout engine in a prior pass (e.g. Taffy).  Used to
    /// distribute auto column widths proportionally to content width rather
    /// than equally.  Return `0.0` for mock/test trees.
    fn cell_content_width(&self, _id: Self::NodeId) -> f32 {
        0.0
    }

    /// Returns the min-content border-box width of cell `id`: the width of its widest unbreakable
    /// content - its longest word, or a replaced element's full width.
    ///
    /// A column is never given less than this. Below it the shaper has to break inside a word,
    /// which browsers do not do under `overflow-wrap: normal`; the content overflows the cell
    /// instead, which is worse. Implementors that cannot measure it may return `0.0`, which
    /// restores the unfloored proportional split.
    fn cell_min_content_width(&mut self, _id: Self::NodeId) -> f32 {
        0.0
    }
}
