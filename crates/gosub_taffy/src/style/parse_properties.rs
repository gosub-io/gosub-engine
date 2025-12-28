use regex::Regex;
use taffy::prelude::*;
use taffy::{Overflow, Point, TextAlign};

use crate::style::parse::{
    parse_align_c, parse_align_i, parse_dimension, parse_grid_auto, parse_grid_placement, parse_len, parse_len_auto,
    parse_tracking_sizing_function,
};
use gosub_interface::config::HasLayouter;
use gosub_interface::css3::CssProperty;
use gosub_interface::layout::LayoutNode;

pub fn parse_display<C: HasLayouter>(node: &impl LayoutNode<C>) -> (Display, crate::Display) {
    let Some(display) = node.get_property("display") else {
        return (Display::Block, crate::Display::Taffy);
    };

    let Some(value) = display.as_string() else {
        return (Display::Block, crate::Display::Taffy);
    };

    match value {
        "none" => (Display::None, crate::Display::Taffy),
        "flex" => (Display::Flex, crate::Display::Taffy),
        "grid" => (Display::Grid, crate::Display::Taffy),
        "inline-block" => (Display::Block, crate::Display::InlineBlock),
        "inline" => (Display::Block, crate::Display::Inline),
        "table" => (Display::Block, crate::Display::Table),
        _ => (Display::Block, crate::Display::Taffy),
    }
}

pub fn parse_overflow<C: HasLayouter>(node: &impl LayoutNode<C>) -> Point<Overflow> {
    fn parse(str: &str) -> Overflow {
        match str {
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
        if let Some(value) = display.as_string() {
            let x = parse(value);
            overflow.x = x;
        }
    }

    if let Some(display) = node.get_property("overflow-y") {
        if let Some(value) = display.as_string() {
            let y = parse(value);
            overflow.y = y;
        }
    }

    overflow
}

pub fn parse_position<C: HasLayouter>(node: &impl LayoutNode<C>) -> Position {
    let Some(position) = node.get_property("position") else {
        return Position::Relative;
    };

    let Some(value) = position.as_string() else {
        return Position::Relative;
    };

    match value {
        "absolute" => Position::Absolute,
        _ => Position::Relative,
    }
}

pub fn parse_inset<C: HasLayouter>(node: &impl LayoutNode<C>) -> Rect<LengthPercentageAuto> {
    Rect {
        top: parse_len_auto(node, "top"),
        right: parse_len_auto(node, "right"),
        bottom: parse_len_auto(node, "bottom"),
        left: parse_len_auto(node, "left"),
    }
}

pub fn parse_size<C: HasLayouter>(node: &impl LayoutNode<C>) -> Size<Dimension> {
    Size {
        width: parse_dimension(node, "width"),
        height: parse_dimension(node, "height"),
    }
}

pub fn parse_min_size<C: HasLayouter>(node: &impl LayoutNode<C>) -> Size<Dimension> {
    Size {
        width: parse_dimension(node, "min-width"),
        height: parse_dimension(node, "min-height"),
    }
}

pub fn parse_max_size<C: HasLayouter>(node: &impl LayoutNode<C>) -> Size<Dimension> {
    Size {
        width: parse_dimension(node, "max-width"),
        height: parse_dimension(node, "max-height"),
    }
}

pub fn parse_aspect_ratio<C: HasLayouter>(node: &impl LayoutNode<C>) -> Option<f32> {
    let aspect_ratio = node.get_property("aspect-ratio")?;

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

pub fn parse_margin<C: HasLayouter>(node: &impl LayoutNode<C>) -> Rect<LengthPercentageAuto> {
    Rect {
        top: parse_len_auto(node, "margin-top"),
        right: parse_len_auto(node, "margin-right"),
        bottom: parse_len_auto(node, "margin-bottom"),
        left: parse_len_auto(node, "margin-left"),
    }
}

pub fn parse_padding<C: HasLayouter>(node: &impl LayoutNode<C>) -> Rect<LengthPercentage> {
    Rect {
        top: parse_len(node, "padding-top"),
        right: parse_len(node, "padding-right"),
        bottom: parse_len(node, "padding-bottom"),
        left: parse_len(node, "padding-left"),
    }
}

pub fn parse_border<C: HasLayouter>(node: &impl LayoutNode<C>) -> Rect<LengthPercentage> {
    Rect {
        top: parse_len(node, "border-top-width"),
        right: parse_len(node, "border-right-width"),
        bottom: parse_len(node, "border-bottom-width"),
        left: parse_len(node, "border-left-width"),
    }
}

pub fn parse_align_items<C: HasLayouter>(node: &impl LayoutNode<C>) -> Option<AlignItems> {
    let display = node.get_property("align-items")?;

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

pub fn parse_align_self<C: HasLayouter>(node: &impl LayoutNode<C>) -> Option<AlignSelf> {
    parse_align_i(node, "align-self")
}

pub fn parse_justify_items<C: HasLayouter>(node: &impl LayoutNode<C>) -> Option<AlignItems> {
    parse_align_i(node, "justify-items")
}

pub fn parse_justify_self<C: HasLayouter>(node: &impl LayoutNode<C>) -> Option<AlignSelf> {
    parse_align_i(node, "justify-self")
}

pub fn parse_align_content<C: HasLayouter>(node: &impl LayoutNode<C>) -> Option<AlignContent> {
    parse_align_c(node, "align-content")
}

pub fn parse_justify_content<C: HasLayouter>(node: &impl LayoutNode<C>) -> Option<JustifyContent> {
    parse_align_c(node, "justify-content")
}

pub fn parse_gap<C: HasLayouter>(node: &impl LayoutNode<C>) -> Size<LengthPercentage> {
    Size {
        width: parse_len(node, "column-gap"),
        height: parse_len(node, "row-gap"),
    }
}

pub fn parse_flex_direction<C: HasLayouter>(node: &impl LayoutNode<C>) -> FlexDirection {
    let Some(property) = node.get_property("flex-direction") else {
        return FlexDirection::Row;
    };

    property.as_string().map_or(FlexDirection::Row, |value| match value {
        "row-reverse" => FlexDirection::RowReverse,
        "column" => FlexDirection::Column,
        "column-reverse" => FlexDirection::ColumnReverse,
        _ => FlexDirection::Row,
    })
}

pub fn parse_flex_wrap<C: HasLayouter>(node: &impl LayoutNode<C>) -> FlexWrap {
    let Some(property) = node.get_property("flex-wrap") else {
        return FlexWrap::NoWrap;
    };

    property.as_string().map_or(FlexWrap::NoWrap, |value| match value {
        "wrap" => FlexWrap::Wrap,
        "wrap-reverse" => FlexWrap::WrapReverse,
        _ => FlexWrap::NoWrap,
    })
}

pub fn parse_flex_basis<C: HasLayouter>(node: &impl LayoutNode<C>) -> Dimension {
    parse_dimension(node, "flex-basis")
}

pub fn parse_flex_grow<C: HasLayouter>(node: &impl LayoutNode<C>) -> f32 {
    let Some(property) = node.get_property("flex-grow") else {
        return 0.0;
    };

    property.as_number().unwrap_or(0.0)
}

pub fn parse_flex_shrink<C: HasLayouter>(node: &impl LayoutNode<C>) -> f32 {
    let Some(property) = node.get_property("flex-shrink") else {
        return 1.0;
    };

    property.as_number().unwrap_or(1.0)
}

pub fn parse_grid_template_rows<C: HasLayouter>(node: &impl LayoutNode<C>) -> Vec<TrackSizingFunction> {
    parse_tracking_sizing_function(node, "grid-template-rows")
}

pub fn parse_grid_template_columns<C: HasLayouter>(node: &impl LayoutNode<C>) -> Vec<TrackSizingFunction> {
    parse_tracking_sizing_function(node, "grid-template-columns")
}

pub fn parse_grid_auto_rows<C: HasLayouter>(node: &impl LayoutNode<C>) -> Vec<NonRepeatedTrackSizingFunction> {
    parse_grid_auto(node, "grid-auto-rows")
}

pub fn parse_grid_auto_columns<C: HasLayouter>(node: &impl LayoutNode<C>) -> Vec<NonRepeatedTrackSizingFunction> {
    parse_grid_auto(node, "grid-auto-columns")
}

pub fn parse_grid_auto_flow<C: HasLayouter>(node: &impl LayoutNode<C>) -> GridAutoFlow {
    let Some(property) = node.get_property("grid-auto-flow") else {
        return GridAutoFlow::Row;
    };

    property.as_string().map_or(GridAutoFlow::Row, |value| match value {
        "column" => GridAutoFlow::Column,
        "row dense" => GridAutoFlow::RowDense,
        "column dense" => GridAutoFlow::ColumnDense,
        _ => GridAutoFlow::Row,
    })
}

pub fn parse_grid_row<C: HasLayouter>(node: &impl LayoutNode<C>) -> Line<GridPlacement> {
    Line {
        start: parse_grid_placement(node, "grid-row-start"),
        end: parse_grid_placement(node, "grid-row-end"),
    }
}

pub fn parse_grid_column<C: HasLayouter>(node: &impl LayoutNode<C>) -> Line<GridPlacement> {
    Line {
        start: parse_grid_placement(node, "grid-column-start"),
        end: parse_grid_placement(node, "grid-column-end"),
    }
}

pub fn parse_box_sizing<C: HasLayouter>(node: &impl LayoutNode<C>) -> BoxSizing {
    let Some(property) = node.get_property("box-sizing") else {
        return BoxSizing::ContentBox;
    };

    property.as_string().map_or(BoxSizing::ContentBox, |value| match value {
        "border-box" => BoxSizing::BorderBox,
        _ => BoxSizing::ContentBox,
    })
}

pub fn parse_text_align<C: HasLayouter>(node: &impl LayoutNode<C>) -> TextAlign {
    let Some(property) = node.get_property("text-align") else {
        return TextAlign::Auto;
    };

    property.as_string().map_or(TextAlign::Auto, |value| match value {
        "-webkit-left" | "-moz-left" => TextAlign::LegacyLeft,
        "-webkit-right" | "-moz-right" => TextAlign::LegacyRight,
        "-webkit-center" | "-moz-center" => TextAlign::LegacyCenter,
        _ => TextAlign::Auto,
    })
}
