use regex::Regex;
use taffy::prelude::*;
use taffy::{Overflow, Point};

use gosub_css3::stylesheet::CssValue;
use gosub_render_backend::RenderBackend;
use gosub_styling::render_tree::{RenderNodeData, RenderTreeNode};

use crate::style::parse::{
    parse_align_c, parse_align_i, parse_dimension, parse_grid_auto, parse_grid_placement,
    parse_len, parse_len_auto, parse_text_dim, parse_tracking_sizing_function,
};

pub(crate) fn parse_display<B: RenderBackend>(node: &mut RenderTreeNode<B>) -> Display {
    let Some(display) = node.get_property("display") else {
        return Display::Block;
    };

    display.compute_value();

    let CssValue::String(ref value) = display.actual else {
        return Display::Block;
    };

    match value.as_str() {
        "none" => Display::None,
        "block" => Display::Block,
        "flex" => Display::Flex,
        "grid" => Display::Grid,
        _ => Display::Block,
    }
}

pub(crate) fn parse_overflow<B: RenderBackend>(node: &mut RenderTreeNode<B>) -> Point<Overflow> {
    fn parse(str: &str) -> Overflow {
        match str {
            "visible" => Overflow::Visible,
            "hidden" => Overflow::Hidden,
            "scroll" => Overflow::Scroll,
            _ => Overflow::Visible,
        }
    }

    let mut overflow = Point {
        x: Overflow::Visible,
        y: Overflow::Visible,
    };

    if let Some(display) = node.get_property("overflow-x") {
        display.compute_value();

        if let CssValue::String(ref value) = display.actual {
            let x = parse(value);
            overflow.x = x;
        };
    };

    if let Some(display) = node.get_property("overflow-y") {
        display.compute_value();

        if let CssValue::String(ref value) = display.actual {
            let y = parse(value);
            overflow.y = y;
        };
    };

    overflow
}

pub(crate) fn parse_position<B: RenderBackend>(node: &mut RenderTreeNode<B>) -> Position {
    let Some(position) = node.get_property("position") else {
        return Position::Relative;
    };

    position.compute_value();

    let CssValue::String(ref value) = position.actual else {
        return Position::Relative;
    };

    match value.as_str() {
        "relative" => Position::Relative,
        "absolute" => Position::Absolute,
        _ => Position::Relative,
    }
}

pub(crate) fn parse_inset<B: RenderBackend>(
    node: &mut RenderTreeNode<B>,
) -> Rect<LengthPercentageAuto> {
    Rect {
        top: parse_len_auto(node, "inset-top"),
        right: parse_len_auto(node, "inset-right"),
        bottom: parse_len_auto(node, "inset-bottom"),
        left: parse_len_auto(node, "inset-left"),
    }
}

pub(crate) fn parse_size<B: RenderBackend>(node: &mut RenderTreeNode<B>) -> Size<Dimension> {
    if let RenderNodeData::Text(t) = &mut node.data {
        return Size {
            width: parse_text_dim(t, "width"),
            height: parse_text_dim(t, "height"),
        };
    }

    Size {
        width: parse_dimension(node, "width"),
        height: parse_dimension(node, "height"),
    }
}

pub(crate) fn parse_min_size<B: RenderBackend>(node: &mut RenderTreeNode<B>) -> Size<Dimension> {
    if let RenderNodeData::Text(t) = &mut node.data {
        return Size {
            width: parse_text_dim(t, "min-width"),
            height: parse_text_dim(t, "min-height"),
        };
    }

    Size {
        width: parse_dimension(node, "min-width"),
        height: parse_dimension(node, "min-height"),
    }
}

pub(crate) fn parse_max_size<B: RenderBackend>(node: &mut RenderTreeNode<B>) -> Size<Dimension> {
    if let RenderNodeData::Text(t) = &mut node.data {
        return Size {
            width: parse_text_dim(t, "max-width"),
            height: parse_text_dim(t, "max-height"),
        };
    }

    Size {
        width: parse_dimension(node, "max-width"),
        height: parse_dimension(node, "max-height"),
    }
}

pub(crate) fn parse_aspect_ratio<B: RenderBackend>(node: &mut RenderTreeNode<B>) -> Option<f32> {
    let aspect_ratio = node.get_property("aspect-ratio")?;

    aspect_ratio.compute_value();

    match &aspect_ratio.actual {
        CssValue::Number(value) => Some(*value),
        CssValue::String(value) => {
            if value == "auto" {
                None
            } else {
                //expecting: number / number
                let Ok(regex) = Regex::new(r"(\d+\.?\d*)\s*/\s*(\d+\.?\d*)") else {
                    return None;
                };
                let captures = regex.captures(value)?;

                if captures.len() != 3 {
                    return None;
                }

                let Ok(numerator) = captures[1].parse::<f32>() else {
                    return None;
                };
                let Ok(denominator) = captures[2].parse::<f32>() else {
                    return None;
                };

                Some(numerator / denominator)
            }
        }

        _ => None,
    }
}

pub(crate) fn parse_margin<B: RenderBackend>(
    node: &mut RenderTreeNode<B>,
) -> Rect<LengthPercentageAuto> {
    Rect {
        top: parse_len_auto(node, "margin-top"),
        right: parse_len_auto(node, "margin-right"),
        bottom: parse_len_auto(node, "margin-bottom"),
        left: parse_len_auto(node, "margin-left"),
    }
}

pub(crate) fn parse_padding<B: RenderBackend>(
    node: &mut RenderTreeNode<B>,
) -> Rect<LengthPercentage> {
    Rect {
        top: parse_len(node, "padding-top"),
        right: parse_len(node, "padding-right"),
        bottom: parse_len(node, "padding-bottom"),
        left: parse_len(node, "padding-left"),
    }
}

pub(crate) fn parse_border<B: RenderBackend>(
    node: &mut RenderTreeNode<B>,
) -> Rect<LengthPercentage> {
    Rect {
        top: parse_len(node, "border-top-width"),
        right: parse_len(node, "border-right-width"),
        bottom: parse_len(node, "border-bottom-width"),
        left: parse_len(node, "border-left-width"),
    }
}

pub(crate) fn parse_align_items<B: RenderBackend>(
    node: &mut RenderTreeNode<B>,
) -> Option<AlignItems> {
    let display = node.get_property("align-items")?;

    display.compute_value();

    let CssValue::String(ref value) = display.actual else {
        return None;
    };

    match value.as_str() {
        "start" => Some(AlignItems::Start),
        "end" => Some(AlignItems::End),
        "flex-start" => Some(AlignItems::FlexStart),
        "flex-end" => Some(AlignItems::FlexEnd),
        "center" => Some(AlignItems::Center),
        "baseline" => Some(AlignItems::Baseline),
        "stretch" => Some(AlignItems::Stretch),
        _ => None,
    }
}

pub(crate) fn parse_align_self<B: RenderBackend>(
    node: &mut RenderTreeNode<B>,
) -> Option<AlignSelf> {
    parse_align_i(node, "align-self")
}

pub(crate) fn parse_justify_items<B: RenderBackend>(
    node: &mut RenderTreeNode<B>,
) -> Option<AlignItems> {
    parse_align_i(node, "justify-items")
}

pub(crate) fn parse_justify_self<B: RenderBackend>(
    node: &mut RenderTreeNode<B>,
) -> Option<AlignSelf> {
    parse_align_i(node, "justify-self")
}

pub(crate) fn parse_align_content<B: RenderBackend>(
    node: &mut RenderTreeNode<B>,
) -> Option<AlignContent> {
    parse_align_c(node, "align-content")
}

pub(crate) fn parse_justify_content<B: RenderBackend>(
    node: &mut RenderTreeNode<B>,
) -> Option<JustifyContent> {
    parse_align_c(node, "justify-content")
}

pub(crate) fn parse_gap<B: RenderBackend>(node: &mut RenderTreeNode<B>) -> Size<LengthPercentage> {
    Size {
        width: parse_len(node, "column-gap"),
        height: parse_len(node, "row-gap"),
    }
}

pub(crate) fn parse_flex_direction<B: RenderBackend>(
    node: &mut RenderTreeNode<B>,
) -> FlexDirection {
    let Some(property) = node.get_property("flex-direction") else {
        return FlexDirection::Row;
    };

    property.compute_value();

    match &property.actual {
        CssValue::String(value) => match value.as_str() {
            "row" => FlexDirection::Row,
            "row-reverse" => FlexDirection::RowReverse,
            "column" => FlexDirection::Column,
            "column-reverse" => FlexDirection::ColumnReverse,
            _ => FlexDirection::Row,
        },
        _ => FlexDirection::Row,
    }
}

pub(crate) fn parse_flex_wrap<B: RenderBackend>(node: &mut RenderTreeNode<B>) -> FlexWrap {
    let Some(property) = node.get_property("flex-wrap") else {
        return FlexWrap::NoWrap;
    };

    property.compute_value();

    match &property.actual {
        CssValue::String(value) => match value.as_str() {
            "nowrap" => FlexWrap::NoWrap,
            "wrap" => FlexWrap::Wrap,
            "wrap-reverse" => FlexWrap::WrapReverse,
            _ => FlexWrap::NoWrap,
        },
        _ => FlexWrap::NoWrap,
    }
}

pub(crate) fn parse_flex_basis<B: RenderBackend>(node: &mut RenderTreeNode<B>) -> Dimension {
    parse_dimension(node, "flex-basis")
}

pub(crate) fn parse_flex_grow<B: RenderBackend>(node: &mut RenderTreeNode<B>) -> f32 {
    let Some(property) = node.get_property("flex-grow") else {
        return 0.0;
    };

    property.compute_value();

    match &property.actual {
        CssValue::Number(value) => *value,
        _ => 0.0,
    }
}

pub(crate) fn parse_flex_shrink<B: RenderBackend>(node: &mut RenderTreeNode<B>) -> f32 {
    let Some(property) = node.get_property("flex-shrink") else {
        return 1.0;
    };

    property.compute_value();

    match &property.actual {
        CssValue::Number(value) => *value,
        _ => 1.0,
    }
}

pub(crate) fn parse_grid_template_rows<B: RenderBackend>(
    node: &mut RenderTreeNode<B>,
) -> Vec<TrackSizingFunction> {
    parse_tracking_sizing_function(node, "grid-template-rows")
}

pub(crate) fn parse_grid_template_columns<B: RenderBackend>(
    node: &mut RenderTreeNode<B>,
) -> Vec<TrackSizingFunction> {
    parse_tracking_sizing_function(node, "grid-template-columns")
}

pub(crate) fn parse_grid_auto_rows<B: RenderBackend>(
    node: &mut RenderTreeNode<B>,
) -> Vec<NonRepeatedTrackSizingFunction> {
    parse_grid_auto(node, "grid-auto-rows")
}

pub(crate) fn parse_grid_auto_columns<B: RenderBackend>(
    node: &mut RenderTreeNode<B>,
) -> Vec<NonRepeatedTrackSizingFunction> {
    parse_grid_auto(node, "grid-auto-columns")
}

pub(crate) fn parse_grid_auto_flow<B: RenderBackend>(node: &mut RenderTreeNode<B>) -> GridAutoFlow {
    let Some(property) = node.get_property("grid-auto-flow") else {
        return GridAutoFlow::Row;
    };

    property.compute_value();

    match &property.actual {
        CssValue::String(value) => match value.as_str() {
            "row" => GridAutoFlow::Row,
            "column" => GridAutoFlow::Column,
            "row dense" => GridAutoFlow::RowDense,
            "column dense" => GridAutoFlow::ColumnDense,
            _ => GridAutoFlow::Row,
        },
        _ => GridAutoFlow::Row,
    }
}

pub(crate) fn parse_grid_row<B: RenderBackend>(
    node: &mut RenderTreeNode<B>,
) -> Line<GridPlacement> {
    Line {
        start: parse_grid_placement(node, "grid-row-start"),
        end: parse_grid_placement(node, "grid-row-end"),
    }
}

pub(crate) fn parse_grid_column<B: RenderBackend>(
    node: &mut RenderTreeNode<B>,
) -> Line<GridPlacement> {
    Line {
        start: parse_grid_placement(node, "grid-column-start"),
        end: parse_grid_placement(node, "grid-column-end"),
    }
}
