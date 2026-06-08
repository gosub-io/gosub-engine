use std::hash::Hash;

use taffy::{
    compute_block_layout, AvailableSpace, Layout as TaffyLayout, LayoutInput, Line, NodeId as TaffyId,
    Point as TaffyPoint, Rect as TaffyRect, RequestedAxis, RunMode, Size as TaffySize, SizingMode,
};

use gosub_interface::config::HasLayouter;
use gosub_interface::css3::CssProperty;
use gosub_interface::layout::{LayoutNode, LayoutTree};
use gosub_lattice::{BoxEdges, CellLayout, CssLength, CssProp, TableRole, TableTree};

use crate::LayoutDocument;

impl<C> TableTree for LayoutDocument<'_, C>
where
    C: HasLayouter<Layouter = crate::TaffyLayouter>,
    <C::LayoutTree as LayoutTree<C>>::NodeId: Hash + Eq,
{
    type NodeId = <C::LayoutTree as LayoutTree<C>>::NodeId;

    fn children(&self, id: Self::NodeId) -> Vec<Self::NodeId> {
        self.tree.children(id).unwrap_or_default()
    }

    fn table_role(&self, id: Self::NodeId) -> TableRole {
        let Some(node) = self.tree.get_node(id) else {
            return TableRole::Other;
        };
        let Some(prop) = node.get_property("display") else {
            return TableRole::Other;
        };
        let Some(value) = prop.as_string() else {
            return TableRole::Other;
        };
        match value {
            "table" => TableRole::Table,
            "table-caption" => TableRole::Caption,
            "table-column-group" => TableRole::ColumnGroup,
            "table-column" => TableRole::Column,
            "table-header-group" => TableRole::HeaderGroup,
            "table-footer-group" => TableRole::FooterGroup,
            "table-row-group" => TableRole::RowGroup,
            "table-row" => TableRole::Row,
            "table-cell" => TableRole::Cell,
            _ => TableRole::Other,
        }
    }

    fn css_length(&self, id: Self::NodeId, prop: CssProp) -> CssLength {
        let prop_name = match prop {
            CssProp::Width => "width",
            CssProp::Height => "height",
            CssProp::MinWidth => "min-width",
            CssProp::MinHeight => "min-height",
            CssProp::MaxWidth => "max-width",
            CssProp::MaxHeight => "max-height",
            CssProp::BorderTopWidth => "border-top-width",
            CssProp::BorderRightWidth => "border-right-width",
            CssProp::BorderBottomWidth => "border-bottom-width",
            CssProp::BorderLeftWidth => "border-left-width",
            CssProp::PaddingTop => "padding-top",
            CssProp::PaddingRight => "padding-right",
            CssProp::PaddingBottom => "padding-bottom",
            CssProp::PaddingLeft => "padding-left",
            CssProp::BorderCollapse => "border-collapse",
            CssProp::BorderSpacingX | CssProp::BorderSpacingY => "border-spacing",
            CssProp::TableLayout => "table-layout",
            CssProp::VerticalAlign => "vertical-align",
            CssProp::CaptionSide => "caption-side",
        };

        let Some(node) = self.tree.get_node(id) else {
            return CssLength::Auto;
        };
        let Some(css_prop) = node.get_property(prop_name) else {
            return CssLength::Auto;
        };

        if let Some((value, unit)) = css_prop.as_unit() {
            if unit == "%" {
                return CssLength::Percent(value);
            }
            return CssLength::Px(css_prop.unit_to_px());
        }
        if let Some(pct) = css_prop.as_percentage() {
            return CssLength::Percent(pct);
        }
        if let Some(s) = css_prop.as_string() {
            if s == "auto" {
                return CssLength::Auto;
            }
        }

        CssLength::Auto
    }

    fn attr_usize(&self, id: Self::NodeId, attr: &str) -> Option<usize> {
        let node = self.tree.get_node(id)?;
        node.get_attribute(attr)?.parse::<usize>().ok()
    }

    fn set_layout(&mut self, id: Self::NodeId, cell_layout: CellLayout) {
        let taffy_layout = TaffyLayout {
            order: 0,
            location: TaffyPoint {
                x: cell_layout.position.x,
                y: cell_layout.position.y,
            },
            size: TaffySize {
                width: cell_layout.size.width,
                height: cell_layout.size.height,
            },
            content_size: TaffySize {
                width: cell_layout.size.width,
                height: cell_layout.size.height,
            },
            scrollbar_size: TaffySize::ZERO,
            border: TaffyRect {
                top: cell_layout.border.top,
                right: cell_layout.border.right,
                bottom: cell_layout.border.bottom,
                left: cell_layout.border.left,
            },
            padding: TaffyRect {
                top: cell_layout.padding.top,
                right: cell_layout.padding.right,
                bottom: cell_layout.padding.bottom,
                left: cell_layout.padding.left,
            },
            margin: TaffyRect::zero(),
        };
        self.tree.set_layout(id, crate::Layout(taffy_layout));
    }

    fn layout_cell(&mut self, id: Self::NodeId, available_width: f32) -> f32 {
        let inputs = LayoutInput {
            run_mode: RunMode::PerformLayout,
            sizing_mode: SizingMode::InherentSize,
            known_dimensions: TaffySize {
                width: Some(available_width),
                height: None,
            },
            parent_size: TaffySize {
                width: Some(available_width),
                height: None,
            },
            available_space: TaffySize {
                width: AvailableSpace::Definite(available_width),
                height: AvailableSpace::MaxContent,
            },
            axis: RequestedAxis::Both,
            vertical_margins_are_collapsible: Line::FALSE,
        };
        let output = compute_block_layout(self, TaffyId::from(id.into()), inputs, None);
        output.size.height
    }
}

/// Unused zero-edges constant kept for potential future use.
#[allow(dead_code)]
pub(crate) const ZERO_EDGES: BoxEdges = BoxEdges {
    top: 0.0,
    right: 0.0,
    bottom: 0.0,
    left: 0.0,
};
