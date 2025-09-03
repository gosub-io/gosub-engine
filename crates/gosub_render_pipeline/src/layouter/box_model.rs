use crate::common::geo;

/// Represents the thickness (or spacing) on each side.
#[derive(Debug, Clone, Copy)]
pub struct Edges {
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
}

/// Represents a boxmodel of an element.
#[derive(Clone, Copy)]
pub struct BoxModel {
    pub content_box: geo::Rect,
    pub padding_box: geo::Rect,
    pub border_box: geo::Rect,
    pub margin_box: geo::Rect,
    /// Thickness of the padding on each side.
    pub padding: Edges,
    /// Thickness of the border on each side.
    pub border: Edges,
    /// Thickness of the margin on each side.
    pub margin: Edges,
}

impl BoxModel {
    pub const ZERO: Self = Self {
        content_box: geo::Rect { x: 0.0, y: 0.0, width: 0.0, height: 0.0 },
        padding_box: geo::Rect { x: 0.0, y: 0.0, width: 0.0, height: 0.0 },
        border_box: geo::Rect { x: 0.0, y: 0.0, width: 0.0, height: 0.0 },
        margin_box: geo::Rect { x: 0.0, y: 0.0, width: 0.0, height: 0.0 },
        padding: Edges { top: 0.0, right: 0.0, bottom: 0.0, left: 0.0 },
        border: Edges { top: 0.0, right: 0.0, bottom: 0.0, left: 0.0 },
        margin: Edges { top: 0.0, right: 0.0, bottom: 0.0, left: 0.0 },
    };

    pub fn new(
        border_box: geo::Rect,
        padding: Edges,
        border: Edges,
        margin: Edges,
    ) -> Self {
        let margin_box = geo::Rect {
            x: border_box.x - margin.left,
            y: border_box.y - margin.top,
            width: border_box.width + margin.left + margin.right,
            height: border_box.height - margin.top + margin.bottom,
        };
        let border_box = border_box;
        let padding_box = geo::Rect {
            x: border_box.x + border.left,
            y: border_box.y + border.top,
            width: border_box.width - border.left - border.right,
            height: border_box.height - border.top - border.bottom,
        };
        let content_box = geo::Rect {
            x: padding_box.x + padding.left,
            y: padding_box.y + padding.top,
            width: padding_box.width - padding.left - padding.right,
            height: padding_box.height - padding.top - padding.bottom,
        };

        Self {
            content_box,
            padding_box,
            border_box,
            margin_box,
            padding,
            border,
            margin,
        }
    }
}

impl std::fmt::Debug for BoxModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mb = self.margin_box;
        let cb = self.content_box;
        let pb = self.padding_box;
        let bb = self.border_box;

        f.debug_struct("BoxModel")
            .field("margin_box", &format_args!("[{}, {}, {}, {}]", mb.x, mb.y, mb.width, mb.height))
            .field("border_box", &format_args!("[{}, {}, {}, {}]", bb.x, bb.y, bb.width, bb.height))
            .field("padding_box", &format_args!("[{}, {}, {}, {}]", pb.x, pb.y, pb.width, pb.height))
            .field("content_box", &format_args!("[{}, {}, {}, {}]", cb.x, cb.y, cb.width, cb.height))
            .field("padding", &format_args!("[{}, {}, {}, {}]", self.padding.top, self.padding.right, self.padding.bottom, self.padding.left))
            .field("border", &format_args!("[{}, {}, {}, {}]", self.border.top, self.border.right, self.border.bottom, self.border.left))
            .field("margin", &format_args!("[{}, {}, {}, {}]", self.margin.top, self.margin.right, self.margin.bottom, self.margin.left))
            .finish()
    }
}