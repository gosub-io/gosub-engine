use crate::geo::{Point, Size};

/// The CSS display role a node plays in the table layout algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableRole {
    /// `display: table` - the outer table box
    Table,
    /// `display: table-caption`
    Caption,
    /// `display: table-column-group`
    ColumnGroup,
    /// `display: table-column`
    Column,
    /// `display: table-header-group` (thead)
    HeaderGroup,
    /// `display: table-footer-group` (tfoot)
    FooterGroup,
    /// `display: table-row-group` (tbody)
    RowGroup,
    /// `display: table-row` (tr)
    Row,
    /// `display: table-cell` (td, th)
    Cell,
    /// Everything else - treated as anonymous cell content
    Other,
}

/// CSS length value resolved to a concrete unit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CssLength {
    Auto,
    Px(f64),
    Percent(f64),
    Zero,
}

impl CssLength {
    /// Resolve to pixels given a `percentage_basis`.  Returns `None` for `auto`.
    pub fn resolve(self, percentage_basis: f64) -> Option<f64> {
        match self {
            CssLength::Auto => None,
            CssLength::Px(px) => Some(px),
            CssLength::Percent(p) => Some(p / 100.0 * percentage_basis),
            CssLength::Zero => Some(0.0),
        }
    }

    pub fn is_auto(self) -> bool {
        matches!(self, CssLength::Auto)
    }

    /// Returns the pixel value, or `default` if not a definite length.
    pub fn px_or(self, default: f64) -> f64 {
        match self {
            CssLength::Px(v) => v,
            CssLength::Zero => 0.0,
            _ => default,
        }
    }
}

/// CSS properties that the table algorithm reads from nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CssProp {
    Width,
    Height,
    MinWidth,
    MinHeight,
    MaxWidth,
    MaxHeight,
    BorderTopWidth,
    BorderRightWidth,
    BorderBottomWidth,
    BorderLeftWidth,
    PaddingTop,
    PaddingRight,
    PaddingBottom,
    PaddingLeft,
    /// Margins, read for the caption only.
    MarginTop,
    MarginRight,
    MarginBottom,
    MarginLeft,
    /// `border-collapse`: `collapse` | `separate`
    BorderCollapse,
    /// Horizontal component of `border-spacing`
    BorderSpacingX,
    /// Vertical component of `border-spacing`
    BorderSpacingY,
    /// `table-layout`: `fixed` | `auto`
    TableLayout,
    /// `vertical-align` on cells
    VerticalAlign,
    /// `caption-side`: `top` | `bottom`
    CaptionSide,
    /// `box-sizing` on the table: `border-box` | `content-box`
    BoxSizing,
}

/// Inset values for a single box edge (top / right / bottom / left), in pixels.
#[derive(Debug, Clone, Copy, Default)]
pub struct BoxEdges {
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
}

impl BoxEdges {
    pub fn horizontal(self) -> f64 {
        self.left + self.right
    }

    pub fn vertical(self) -> f64 {
        self.top + self.bottom
    }
}

/// `vertical-align` on a table cell - the subset that applies to cell boxes.
/// `baseline` (the CSS initial) is approximated as `Top` until real first-line
/// baseline metrics are available.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VerticalAlign {
    #[default]
    Top,
    Middle,
    Bottom,
    /// Align the cell's first in-flow line baseline with the row baseline
    /// (CSS 2 §17.5.3). Cells without a line box fall back to Top.
    Baseline,
}

/// The computed layout written back to the tree for each table-internal node.
#[derive(Debug, Clone)]
pub struct CellLayout {
    /// Position relative to the node's parent in the tree.
    pub position: Point,
    /// Border-box size (content + padding + border).
    pub size: Size,
    pub border: BoxEdges,
    pub padding: BoxEdges,
    /// Vertical offset the host should add to the cell's *children* (relative
    /// to the content-box top) to realize `vertical-align` when the cell is
    /// taller than its content. Always 0 for non-cell nodes.
    pub content_offset_y: f64,
}

impl Default for CellLayout {
    fn default() -> Self {
        Self {
            position: Point::new(0.0, 0.0),
            size: Size::new(0.0, 0.0),
            border: BoxEdges::default(),
            padding: BoxEdges::default(),
            content_offset_y: 0.0,
        }
    }
}

/// `table-layout` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TableSizing {
    /// `table-layout: auto` - column widths derived from content.
    #[default]
    Auto,
    /// `table-layout: fixed` - column widths from first row / `<col>` elements.
    Fixed,
}

/// One background layer under a cell (css-tables-3 §5.3.2): the column group, column, row group
/// or row the cell originates in, with that part's box relative to the table. The background is
/// positioned against this box and shows only inside the cell.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellLayer<N> {
    pub node: N,
    pub position: Point,
    pub size: Size,
}

/// Per-cell result of border-collapse conflict resolution.
#[derive(Debug, Clone, Copy, Default)]
pub struct CollapsedBorders {
    /// The layout border: HALF the widest resolved boundary along each edge
    /// (collapsed borders are centered on the grid lines, so each adjacent
    /// cell's geometry carries half; outer table edges carry half too, the
    /// other half sticking out of the table box like in browsers).
    pub layout: BoxEdges,
}

/// `border-style`, ordered by its weight in border-conflict resolution (CSS 2 §17.6.2.1):
/// `hidden` beats everything, `none` loses to everything, and between equally wide
/// borders the later variant wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BorderStyle {
    None,
    Inset,
    Groove,
    Outset,
    Ridge,
    Dotted,
    Dashed,
    Solid,
    Double,
    Hidden,
}

/// One side of a box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Top,
    Right,
    Bottom,
    Left,
}

impl Side {
    pub const ALL: [Side; 4] = [Side::Top, Side::Right, Side::Bottom, Side::Left];
}

/// The border that won a collapsed boundary: paint it with this node's `side` border
/// color and style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BorderOwner<N> {
    pub node: N,
    pub side: Side,
}

/// A stretch of a collapsed cell edge between two grid lines that cross it, with its own
/// conflict winner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdgeSegment<N> {
    /// Start and end along the edge, from the cell's top-left border-box corner.
    pub start: f64,
    pub end: f64,
    /// The cell's half of the boundary, painted inside its border box.
    pub width: f64,
    /// Painted outside the border box: the other half, where no cell sits across the line (the
    /// table perimeter, an empty row).
    pub outset: f64,
    /// `None` when nothing is painted (a `hidden` border won, or no border at all).
    pub owner: Option<BorderOwner<N>>,
}

/// A slot of a collapsed table that no cell covers. Its row paints the slot's share of the
/// lines around it, the way a cell would.
#[derive(Debug, Clone, PartialEq)]
pub struct EmptySlotPaint<N> {
    /// Left edge and width of the slot, relative to the row's border box.
    pub x: f64,
    pub width: f64,
    pub paint: CollapsedPaint<N>,
}

/// How a collapsed cell paints its share of the table's borders. Each cell paints its own
/// half of every boundary, so the result does not depend on the order cells are painted in.
#[derive(Debug, Clone, PartialEq)]
pub struct CollapsedPaint<N> {
    /// Segments along each edge, `[top, right, bottom, left]`, in increasing offset.
    pub edges: [Vec<EdgeSegment<N>>; 4],
    /// Winner of the joint at each corner, `[top-left, top-right, bottom-right,
    /// bottom-left]`: the cell's quarter of the joint is painted in its style.
    pub corners: [Option<BorderOwner<N>>; 4],
    /// How far each corner's quarter of the joint reaches outside the border box, `[x, y]`:
    /// as far as the widest line through the joint, where nothing is painted across.
    pub corner_outsets: [[f64; 2]; 4],
}

/// `border-collapse` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BorderCollapse {
    /// Adjacent borders are kept separate, with `border-spacing` between cells.
    #[default]
    Separate,
    /// Adjacent borders are merged; the "winning" border is rendered.
    Collapse,
}
