use taffy::prelude::*;
use taffy::{
    AlignContent, AlignItems, Dimension, GridPlacement, LengthPercentage, LengthPercentageAuto,
    TrackSizingFunction,
};

use gosub_styling::styling::CssValue;
use gosub_styling::render_tree::{RenderNodeData, RenderTreeNode};

pub(crate) fn parse_len(node: &mut RenderTreeNode, name: &str) -> LengthPercentage {
    let Some(property) = node.get_property(name) else {
        return LengthPercentage::Length(0.0);
    };

    property.compute_value();

    match &property.actual {
        CssValue::Percentage(value) => LengthPercentage::Percent(*value),
        CssValue::Unit(..) => LengthPercentage::Length(property.actual.unit_to_px()),
        CssValue::String(_) => LengthPercentage::Length(property.actual.unit_to_px()), //HACK
        _ => LengthPercentage::Length(0.0),
    }
}

pub(crate) fn parse_len_auto(node: &mut RenderTreeNode, name: &str) -> LengthPercentageAuto {
    let Some(property) = node.get_property(name) else {
        return LengthPercentageAuto::Auto;
    };

    property.compute_value();

    match &property.actual {
        CssValue::String(value) => match value.as_str() {
            "auto" => LengthPercentageAuto::Auto,
            _ => LengthPercentageAuto::Length(property.actual.unit_to_px()), //HACK
        },
        CssValue::Percentage(value) => LengthPercentageAuto::Percent(*value),
        CssValue::Unit(..) => LengthPercentageAuto::Length(property.actual.unit_to_px()),
        _ => LengthPercentageAuto::Auto,
    }
}

pub(crate) fn parse_dimension(node: &mut RenderTreeNode, name: &str) -> Dimension {
    let mut auto = Dimension::Auto;
    if let RenderNodeData::Text(text) = &node.data {
        if name == "width" {
            auto = Dimension::Length(text.width);
        } else if name == "height" {
            auto = Dimension::Length(text.height);
        }
    }

    let Some(property) = node.get_property(name) else {
        return auto;
    };

    property.compute_value();

    if name == "width" {
        println!("Width: {:?}", property.actual);
    }

    match &property.actual {
        CssValue::String(value) => match value.as_str() {
            "auto" => auto,
            s if s.ends_with('%') => {
                let value = s.trim_end_matches('%').parse::<f32>().unwrap_or(0.0);
                Dimension::Percent(value)
            }
            _ => Dimension::Length(property.actual.unit_to_px()), //HACK
        },
        CssValue::Percentage(value) => Dimension::Percent(*value),
        CssValue::Unit(..) => Dimension::Length(property.actual.unit_to_px()),
        _ => auto,
    }
}

pub(crate) fn parse_align_i(node: &mut RenderTreeNode, name: &str) -> Option<AlignItems> {
    let display = node.get_property(name)?;
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

pub(crate) fn parse_align_c(node: &mut RenderTreeNode, name: &str) -> Option<AlignContent> {
    let display = node.get_property(name)?;

    display.compute_value();

    let CssValue::String(ref value) = display.actual else {
        return None;
    };

    match value.as_str() {
        "start" => Some(AlignContent::Start),
        "end" => Some(AlignContent::End),
        "flex-start" => Some(AlignContent::FlexStart),
        "flex-end" => Some(AlignContent::FlexEnd),
        "center" => Some(AlignContent::Center),
        "stretch" => Some(AlignContent::Stretch),
        "space-between" => Some(AlignContent::SpaceBetween),
        "space-around" => Some(AlignContent::SpaceAround),
        _ => None,
    }
}

pub(crate) fn parse_tracking_sizing_function(
    node: &mut RenderTreeNode,
    name: &str,
) -> Vec<TrackSizingFunction> {
    let Some(display) = node.get_property(name) else {
        return Vec::new();
    };

    display.compute_value();

    let CssValue::String(ref _value) = display.actual else {
        return Vec::new();
    };

    Vec::new() //TODO: Implement this
}

#[allow(dead_code)]
pub(crate) fn parse_non_repeated_tracking_sizing_function(
    _node: &mut RenderTreeNode,
    _name: &str,
) -> NonRepeatedTrackSizingFunction {
    todo!("implement parse_non_repeated_tracking_sizing_function")
}

pub(crate) fn parse_grid_auto(
    node: &mut RenderTreeNode,
    name: &str,
) -> Vec<NonRepeatedTrackSizingFunction> {
    let Some(display) = node.get_property(name) else {
        return Vec::new();
    };

    display.compute_value();

    let CssValue::String(ref _value) = display.actual else {
        return Vec::new();
    };

    Vec::new() //TODO: Implement this
}

pub(crate) fn parse_grid_placement(node: &mut RenderTreeNode, name: &str) -> GridPlacement {
    let Some(display) = node.get_property(name) else {
        return GridPlacement::Auto;
    };

    display.compute_value();

    match &display.actual {
        CssValue::String(value) => {
            if value.starts_with("span") {
                let value = value.trim_start_matches("span").trim();

                if let Ok(value) = value.parse::<u16>() {
                    GridPlacement::from_span(value)
                } else {
                    GridPlacement::Auto
                }
            } else if let Ok(value) = value.parse::<i16>() {
                GridPlacement::from_line_index(value)
            } else {
                GridPlacement::Auto
            }
        }
        CssValue::Number(value) => GridPlacement::from_line_index(*value as i16),
        _ => GridPlacement::Auto,
    }
}
