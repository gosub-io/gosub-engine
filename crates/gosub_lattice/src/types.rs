use gosub_shared::geo::{Point, Size};

/// The CSS display role a node plays in the table layout algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableRole {
    /// `display: table` — the outer table box
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
    /// Everything else — treated as anonymous cell content
    Other,
}

/// CSS length value resolved to a concrete unit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CssLength {
    Auto,
    Px(f32),
    Percent(f32),
    Zero,
}

impl CssLength {
    /// Resolve to pixels given a `percentage_basis`.  Returns `None` for `auto`.
    pub fn resolve(self, percentage_basis: f32) -> Option<f32> {
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
    pub fn px_or(self, default: f32) -> f32 {
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
}

/// Inset values for a single box edge (top / right / bottom / left), in pixels.
#[derive(Debug, Clone, Copy, Default)]
pub struct BoxEdges {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl BoxEdges {
    pub fn horizontal(self) -> f32 {
        self.left + self.right
    }

    pub fn vertical(self) -> f32 {
        self.top + self.bottom
    }
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
}

impl Default for CellLayout {
    fn default() -> Self {
        Self {
            position: Point::new(0.0, 0.0),
            size: Size::new(0.0, 0.0),
            border: BoxEdges::default(),
            padding: BoxEdges::default(),
        }
    }
}

/// `table-layout` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TableSizing {
    /// `table-layout: auto` — column widths derived from content.
    #[default]
    Auto,
    /// `table-layout: fixed` — column widths from first row / `<col>` elements.
    Fixed,
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
