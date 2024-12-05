use taffy::style_helpers::{TaffyGridLine, TaffyGridSpan};
use taffy::{
    AlignContent, AlignItems, Dimension, GridPlacement, LengthPercentage, LengthPercentageAuto,
    NonRepeatedTrackSizingFunction, TrackSizingFunction,
};

use gosub_shared::render_backend::geo::Size;
use gosub_shared::render_backend::layout::LayoutNode;
use gosub_shared::traits::config::HasLayouter;
use gosub_shared::traits::css3::CssProperty;

pub fn parse_len<C: HasLayouter>(node: &mut impl LayoutNode<C>, name: &str) -> LengthPercentage {
    let Some(property) = node.get_property(name) else {
        return LengthPercentage::Length(0.0);
    };

    if let Some(percent) = property.as_percentage() {
        return LengthPercentage::Percent(percent / 100.0);
    }

    LengthPercentage::Length(property.unit_to_px())
}

pub fn parse_len_auto<C: HasLayouter>(node: &mut impl LayoutNode<C>, name: &str) -> LengthPercentageAuto {
    let Some(property) = node.get_property(name) else {
        return LengthPercentageAuto::Length(0.0);
    };

    if let Some(str) = property.as_string() {
        if str == "auto" {
            return LengthPercentageAuto::Auto;
        }
    }

    if let Some(percent) = property.as_percentage() {
        return LengthPercentageAuto::Percent(percent / 100.0);
    }

    LengthPercentageAuto::Length(property.unit_to_px())
}

pub fn parse_dimension<C: HasLayouter>(node: &mut impl LayoutNode<C>, name: &str) -> Dimension {
    let Some(property) = node.get_property(name) else {
        return Dimension::Auto;
    };

    if let Some(str) = property.as_string() {
        if str == "auto" {
            return Dimension::Auto;
        }
    }

    if let Some(percent) = property.as_percentage() {
        return Dimension::Percent(percent / 100.0);
    }

    Dimension::Length(property.unit_to_px())
}

pub fn parse_text_dim(size: Size, name: &str) -> Dimension {
    if name == "width" || name == "max-width" || name == "min-width" {
        Dimension::Length(size.width)
    } else if name == "height" || name == "max-height" || name == "min-height" {
        Dimension::Length(size.height)
    } else {
        Dimension::Auto
    }
}

pub fn parse_align_i<C: HasLayouter>(node: &mut impl LayoutNode<C>, name: &str) -> Option<AlignItems> {
    let display = node.get_property(name)?;
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

pub fn parse_align_c<C: HasLayouter>(node: &mut impl LayoutNode<C>, name: &str) -> Option<AlignContent> {
    let display = node.get_property(name)?;

    let value = display.as_string()?;

    match value {
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

pub fn parse_tracking_sizing_function<C: HasLayouter>(
    node: &mut impl LayoutNode<C>,
    name: &str,
) -> Vec<TrackSizingFunction> {
    let Some(display) = node.get_property(name) else {
        return Vec::new();
    };

    let Some(_value) = display.as_string() else {
        return Vec::new();
    };

    Vec::new() //TODO: Implement this
}

#[allow(dead_code)]
pub fn parse_non_repeated_tracking_sizing_function<C: HasLayouter>(
    _node: &mut impl LayoutNode<C>,
    _name: &str,
) -> NonRepeatedTrackSizingFunction {
    todo!("implement parse_non_repeated_tracking_sizing_function")
}

pub fn parse_grid_auto<C: HasLayouter>(
    node: &mut impl LayoutNode<C>,
    name: &str,
) -> Vec<NonRepeatedTrackSizingFunction> {
    let Some(display) = node.get_property(name) else {
        return Vec::new();
    };

    let Some(_value) = display.as_string() else {
        return Vec::new();
    };

    Vec::new() //TODO: Implement this
}

pub fn parse_grid_placement<C: HasLayouter>(node: &mut impl LayoutNode<C>, name: &str) -> GridPlacement {
    let Some(display) = node.get_property(name) else {
        return GridPlacement::Auto;
    };

    if let Some(value) = &display.as_string() {
        return if value.starts_with("span") {
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
        };
    }

    if let Some(value) = display.as_number() {
        return GridPlacement::from_line_index(value as i16);
    }
    GridPlacement::Auto
}
