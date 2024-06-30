use taffy::prelude::*;
use taffy::{
    AlignContent, AlignItems, Dimension, GridPlacement, LengthPercentage, LengthPercentageAuto,
    TrackSizingFunction,
};

use gosub_render_backend::{PreRenderText, RenderBackend};
// use gosub_styling::css_values::CssValue;
use gosub_css3::stylesheet::CssValue;
use gosub_styling::render_tree::{RenderTreeNode, TextData};

pub(crate) fn parse_len<B: RenderBackend>(
    node: &mut RenderTreeNode<B>,
    name: &str,
) -> LengthPercentage {
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

pub(crate) fn parse_len_auto<B: RenderBackend>(
    node: &mut RenderTreeNode<B>,
    name: &str,
) -> LengthPercentageAuto {
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

pub(crate) fn parse_dimension<B: RenderBackend>(
    node: &mut RenderTreeNode<B>,
    name: &str,
) -> Dimension {
    let Some(property) = node.get_property(name) else {
        return Dimension::Auto;
    };

    property.compute_value();

    match &property.actual {
        CssValue::String(value) => match value.as_str() {
            "auto" => Dimension::Auto,
            s if s.ends_with('%') => {
                let value = s.trim_end_matches('%').parse::<f32>().unwrap_or(0.0);
                Dimension::Percent(value)
            }
            _ => Dimension::Length(property.actual.unit_to_px()), //HACK
        },
        CssValue::Percentage(value) => Dimension::Percent(*value),
        CssValue::Unit(..) => Dimension::Length(property.actual.unit_to_px()),
        _ => Dimension::Auto,
    }
}

pub(crate) fn parse_text_dim<B: RenderBackend>(text: &mut TextData<B>, name: &str) -> Dimension {
    let size = text.prerender.prerender();

    if name == "width" || name == "max-width" || name == "min-width" {
        Dimension::Length(size.width)
    } else if name == "height" || name == "max-height" || name == "min-height" {
        Dimension::Length(size.height)
    } else {
        Dimension::Auto
    }
}

pub(crate) fn parse_align_i<B: RenderBackend>(
    node: &mut RenderTreeNode<B>,
    name: &str,
) -> Option<AlignItems> {
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

pub(crate) fn parse_align_c<B: RenderBackend>(
    node: &mut RenderTreeNode<B>,
    name: &str,
) -> Option<AlignContent> {
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

pub(crate) fn parse_tracking_sizing_function<B: RenderBackend>(
    node: &mut RenderTreeNode<B>,
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
pub(crate) fn parse_non_repeated_tracking_sizing_function<B: RenderBackend>(
    _node: &mut RenderTreeNode<B>,
    _name: &str,
) -> NonRepeatedTrackSizingFunction {
    todo!("implement parse_non_repeated_tracking_sizing_function")
}

pub(crate) fn parse_grid_auto<B: RenderBackend>(
    node: &mut RenderTreeNode<B>,
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

pub(crate) fn parse_grid_placement<B: RenderBackend>(
    node: &mut RenderTreeNode<B>,
    name: &str,
) -> GridPlacement {
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
        CssValue::Number(value) => GridPlacement::from_line_index((*value) as i16),
        _ => GridPlacement::Auto,
    }
}
