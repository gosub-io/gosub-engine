use regex::Regex;
use taffy::prelude::*;
use taffy::{Overflow, Point};

use gosub_render_backend::layout::{CssProperty, Node};

use crate::style::parse::{
    parse_align_c, parse_align_i, parse_dimension, parse_grid_auto, parse_grid_placement,
    parse_len, parse_len_auto, parse_text_dim, parse_tracking_sizing_function,
};

pub fn parse_display(node: &mut impl Node) -> (Display, crate::Display) {
    let Some(display) = node.get_property("display") else {
        return (Display::Block, crate::Display::Taffy);
    };

    display.compute_value();

    let Some(value) = display.as_string() else {
        return (Display::Block, crate::Display::Taffy);
    };

    match value {
        "none" => (Display::None, crate::Display::Taffy),
        "block" => (Display::Block, crate::Display::Taffy),
        "flex" => (Display::Flex, crate::Display::Taffy),
        "grid" => (Display::Grid, crate::Display::Taffy),
        "inline-block" => (Display::Block, crate::Display::InlineBlock),
        "inline" => (Display::Block, crate::Display::Inline),
        "table" => (Display::Block, crate::Display::Table),
        _ => (Display::Block, crate::Display::Taffy),
    }
}

pub fn parse_overflow(node: &mut impl Node) -> Point<Overflow> {
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

        if let Some(value) = display.as_string() {
            let x = parse(value);
            overflow.x = x;
        };
    };

    if let Some(display) = node.get_property("overflow-y") {
        display.compute_value();

        if let Some(value) = display.as_string() {
            let y = parse(value);
            overflow.y = y;
        };
    };

    overflow
}

pub fn parse_position(node: &mut impl Node) -> Position {
    let Some(position) = node.get_property("position") else {
        return Position::Relative;
    };

    position.compute_value();

    let Some(value) = position.as_string() else {
        return Position::Relative;
    };

    match value {
        "relative" => Position::Relative,
        "absolute" => Position::Absolute,
        _ => Position::Relative,
    }
}

pub fn parse_inset(node: &mut impl Node) -> Rect<LengthPercentageAuto> {
    Rect {
        top: parse_len_auto(node, "inset-top"),
        right: parse_len_auto(node, "inset-right"),
        bottom: parse_len_auto(node, "inset-bottom"),
        left: parse_len_auto(node, "inset-left"),
    }
}

pub fn parse_size(node: &mut impl Node) -> Size<Dimension> {
    if let Some(t) = node.text_size() {
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

pub fn parse_min_size(node: &mut impl Node) -> Size<Dimension> {
    if let Some(t) = node.text_size() {
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

pub fn parse_max_size(node: &mut impl Node) -> Size<Dimension> {
    if let Some(t) = node.text_size() {
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

pub fn parse_aspect_ratio(node: &mut impl Node) -> Option<f32> {
    let aspect_ratio = node.get_property("aspect-ratio")?;

    aspect_ratio.compute_value();

    if let Some(value) = aspect_ratio.as_number() {
        return Some(value);
    }

    if let Some(value) = aspect_ratio.as_string() {
        return if value == "auto" {
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
        };
    }

    None
}

pub fn parse_margin(node: &mut impl Node) -> Rect<LengthPercentageAuto> {
    Rect {
        top: parse_len_auto(node, "margin-top"),
        right: parse_len_auto(node, "margin-right"),
        bottom: parse_len_auto(node, "margin-bottom"),
        left: parse_len_auto(node, "margin-left"),
    }
}

pub fn parse_padding(node: &mut impl Node) -> Rect<LengthPercentage> {
    Rect {
        top: parse_len(node, "padding-top"),
        right: parse_len(node, "padding-right"),
        bottom: parse_len(node, "padding-bottom"),
        left: parse_len(node, "padding-left"),
    }
}

pub fn parse_border(node: &mut impl Node) -> Rect<LengthPercentage> {
    Rect {
        top: parse_len(node, "border-top-width"),
        right: parse_len(node, "border-right-width"),
        bottom: parse_len(node, "border-bottom-width"),
        left: parse_len(node, "border-left-width"),
    }
}

pub fn parse_align_items(node: &mut impl Node) -> Option<AlignItems> {
    let display = node.get_property("align-items")?;

    display.compute_value();

    let value = display.as_string()?;

    match value {
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

pub fn parse_align_self(node: &mut impl Node) -> Option<AlignSelf> {
    parse_align_i(node, "align-self")
}

pub fn parse_justify_items(node: &mut impl Node) -> Option<AlignItems> {
    parse_align_i(node, "justify-items")
}

pub fn parse_justify_self(node: &mut impl Node) -> Option<AlignSelf> {
    parse_align_i(node, "justify-self")
}

pub fn parse_align_content(node: &mut impl Node) -> Option<AlignContent> {
    parse_align_c(node, "align-content")
}

pub fn parse_justify_content(node: &mut impl Node) -> Option<JustifyContent> {
    parse_align_c(node, "justify-content")
}

pub fn parse_gap(node: &mut impl Node) -> Size<LengthPercentage> {
    Size {
        width: parse_len(node, "column-gap"),
        height: parse_len(node, "row-gap"),
    }
}

pub fn parse_flex_direction(node: &mut impl Node) -> FlexDirection {
    let Some(property) = node.get_property("flex-direction") else {
        return FlexDirection::Row;
    };

    property.compute_value();

    if let Some(value) = property.as_string() {
        match value {
            "row" => FlexDirection::Row,
            "row-reverse" => FlexDirection::RowReverse,
            "column" => FlexDirection::Column,
            "column-reverse" => FlexDirection::ColumnReverse,
            _ => FlexDirection::Row,
        }
    } else {
        FlexDirection::Row
    }
}

pub fn parse_flex_wrap(node: &mut impl Node) -> FlexWrap {
    let Some(property) = node.get_property("flex-wrap") else {
        return FlexWrap::NoWrap;
    };

    property.compute_value();

    if let Some(value) = property.as_string() {
        match value {
            "nowrap" => FlexWrap::NoWrap,
            "wrap" => FlexWrap::Wrap,
            "wrap-reverse" => FlexWrap::WrapReverse,
            _ => FlexWrap::NoWrap,
        }
    } else {
        FlexWrap::NoWrap
    }
}

pub fn parse_flex_basis(node: &mut impl Node) -> Dimension {
    parse_dimension(node, "flex-basis")
}

pub fn parse_flex_grow(node: &mut impl Node) -> f32 {
    let Some(property) = node.get_property("flex-grow") else {
        return 0.0;
    };

    property.compute_value();

    property.as_number().unwrap_or(0.0)
}

pub fn parse_flex_shrink(node: &mut impl Node) -> f32 {
    let Some(property) = node.get_property("flex-shrink") else {
        return 1.0;
    };

    property.compute_value();

    property.as_number().unwrap_or(1.0)
}

pub fn parse_grid_template_rows(node: &mut impl Node) -> Vec<TrackSizingFunction> {
    parse_tracking_sizing_function(node, "grid-template-rows")
}

pub fn parse_grid_template_columns(node: &mut impl Node) -> Vec<TrackSizingFunction> {
    parse_tracking_sizing_function(node, "grid-template-columns")
}

pub fn parse_grid_auto_rows(node: &mut impl Node) -> Vec<NonRepeatedTrackSizingFunction> {
    parse_grid_auto(node, "grid-auto-rows")
}

pub fn parse_grid_auto_columns(node: &mut impl Node) -> Vec<NonRepeatedTrackSizingFunction> {
    parse_grid_auto(node, "grid-auto-columns")
}

pub fn parse_grid_auto_flow(node: &mut impl Node) -> GridAutoFlow {
    let Some(property) = node.get_property("grid-auto-flow") else {
        return GridAutoFlow::Row;
    };

    property.compute_value();

    if let Some(value) = property.as_string() {
        match value {
            "row" => GridAutoFlow::Row,
            "column" => GridAutoFlow::Column,
            "row dense" => GridAutoFlow::RowDense,
            "column dense" => GridAutoFlow::ColumnDense,
            _ => GridAutoFlow::Row,
        }
    } else {
        GridAutoFlow::Row
    }
}

pub fn parse_grid_row(node: &mut impl Node) -> Line<GridPlacement> {
    Line {
        start: parse_grid_placement(node, "grid-row-start"),
        end: parse_grid_placement(node, "grid-row-end"),
    }
}

pub fn parse_grid_column(node: &mut impl Node) -> Line<GridPlacement> {
    Line {
        start: parse_grid_placement(node, "grid-column-start"),
        end: parse_grid_placement(node, "grid-column-end"),
    }
}
