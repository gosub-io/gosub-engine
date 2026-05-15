use taffy::style_helpers::{TaffyGridLine, TaffyGridSpan};
use taffy::{
    AlignContent, AlignItems, Dimension, GridPlacement, LengthPercentage, LengthPercentageAuto, TrackSizingFunction,
};

use gosub_interface::config::HasLayouter;
use gosub_interface::css3::{CssProperty, CssSystem, CssValue};
use gosub_interface::layout::LayoutNode;

use crate::calc::{self, CalcExpr};

/// Storage for parsed `calc()` expressions tied to a single Taffy `Style`.
///
/// Taffy encodes a calc value as a raw pointer; we keep the underlying boxes alive here so those
/// pointers stay valid for the duration of layout. Heap addresses of the boxed `CalcExpr`s don't
/// move when the vec reallocates — only the `Box` slots themselves do. (`Vec<CalcExpr>` would
/// move elements on reallocation and invalidate the raw pointers we handed to Taffy.)
#[allow(clippy::vec_box)]
pub type CalcStorage = Vec<Box<CalcExpr>>;

/// Box up a parsed calc expression and return a stable, 8-byte-aligned pointer to it.
fn store_calc(storage: &mut CalcStorage, expr: CalcExpr) -> *const () {
    let boxed = Box::new(expr);
    let ptr = (&*boxed) as *const CalcExpr as *const ();
    storage.push(boxed);
    ptr
}

/// If `property` is a `calc(...)` function, parse its body.
fn take_calc<C: HasLayouter>(property: &<C::CssSystem as CssSystem>::Property) -> Option<CalcExpr> {
    let (name, args) = property.as_function()?;
    if !name.eq_ignore_ascii_case("calc") {
        return None;
    }
    let body = args.first().and_then(CssValue::as_string)?;
    calc::parse(body)
}

// Parse functions that will parse a CSS property and converts it into a Taffy type so it can be used
// in the taffy layout engine. This step is needed since our CSS properties are not directly compatible
// with the Taffy layout engine.

pub fn parse_len<C: HasLayouter>(
    node: &mut impl LayoutNode<C>,
    name: &str,
    calc_storage: &mut CalcStorage,
) -> LengthPercentage {
    let Some(property) = node.get_property(name) else {
        return LengthPercentage::length(0.0);
    };

    if let Some(expr) = take_calc::<C>(property) {
        return LengthPercentage::calc(store_calc(calc_storage, expr));
    }

    if let Some(percent) = property.as_percentage() {
        return LengthPercentage::percent(percent / 100.0);
    }

    LengthPercentage::length(property.unit_to_px())
}

pub fn parse_len_auto<C: HasLayouter>(
    node: &mut impl LayoutNode<C>,
    name: &str,
    calc_storage: &mut CalcStorage,
) -> LengthPercentageAuto {
    let Some(property) = node.get_property(name) else {
        return LengthPercentageAuto::length(0.0);
    };

    if let Some(str) = property.as_string() {
        if str == "auto" {
            return LengthPercentageAuto::auto();
        }
    }

    if let Some(expr) = take_calc::<C>(property) {
        return LengthPercentageAuto::calc(store_calc(calc_storage, expr));
    }

    if let Some(percent) = property.as_percentage() {
        return LengthPercentageAuto::percent(percent / 100.0);
    }

    LengthPercentageAuto::length(property.unit_to_px())
}

pub fn parse_dimension<C: HasLayouter>(
    node: &mut impl LayoutNode<C>,
    name: &str,
    calc_storage: &mut CalcStorage,
) -> Dimension {
    let Some(property) = node.get_property(name) else {
        return Dimension::auto();
    };

    if let Some(str) = property.as_string() {
        if str == "auto" {
            return Dimension::auto();
        }
    }

    if let Some(expr) = take_calc::<C>(property) {
        return Dimension::calc(store_calc(calc_storage, expr));
    }

    if let Some(percent) = property.as_percentage() {
        return Dimension::percent(percent / 100.0);
    }

    Dimension::length(property.unit_to_px())
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
) -> TrackSizingFunction {
    todo!("implement parse_non_repeated_tracking_sizing_function")
}

pub fn parse_grid_auto<C: HasLayouter>(node: &mut impl LayoutNode<C>, name: &str) -> Vec<TrackSizingFunction> {
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
