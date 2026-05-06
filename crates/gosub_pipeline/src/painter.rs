pub mod commands;

use crate::common::browser_state::{get_browser_state, BrowserState, WireframeState};
use crate::common::document::node::NodeId;
use crate::common::document::style::{Color as StyleColor, StyleProperty, StyleValue};
use crate::common::get_media_store;
use crate::common::media::{Media, MediaType};
use crate::layering::layer::LayerList;
use crate::layouter::{ElementContext, LayoutElementNode};
use crate::painter::commands::border::{Border, BorderStyle};
use crate::painter::commands::brush::Brush;
use crate::painter::commands::color::Color;
use crate::painter::commands::rectangle::{Radius, Rectangle};
use crate::painter::commands::text::Text;
use crate::painter::commands::PaintCommand;
use crate::tiler::{Tile, TiledLayoutElement};
use rand::Rng;
use std::ops::AddAssign;
use std::sync::Arc;

/// Painter works with the layout tree and generates paint commands for the renderer. It does not
/// generate a new data structure as output, but will update the existing layout elements with
/// paint commands.
pub struct Painter {
    layer_list: Arc<LayerList>,
}

impl Painter {
    pub fn new(layer_list: Arc<LayerList>) -> Painter {
        Painter { layer_list }
    }

    // Generate paint commands for the given tile
    pub fn paint(&self, element: &TiledLayoutElement) -> Vec<PaintCommand> {
        let mut commands = Vec::new();

        let Some(layout_element) = self.layer_list.layout_tree.get_node_by_id(element.id) else {
            return Vec::new();
        };
        let dom_node_id = layout_element.dom_node_id;

        let binding = get_browser_state();
        let state = binding.read().unwrap();

        // Paint boxmodel for the hovered element if needed
        if state.debug_hover
            && state.current_hovered_element.is_some()
            && state.current_hovered_element.unwrap() == layout_element.id
        {
            commands.extend(self.generate_boxmodel_commands(layout_element));
        }

        match state.wireframed {
            WireframeState::Only => {
                commands.extend(self.generate_wireframe_commands(layout_element));
            }
            WireframeState::Both => {
                commands.extend(self.generate_element_commands(layout_element, dom_node_id));
                commands.extend(self.generate_wireframe_commands(layout_element));
            }
            WireframeState::None => {
                commands.extend(self.generate_element_commands(layout_element, dom_node_id));
            }
        }

        commands
    }

    fn get_brush(&self, node_id: NodeId, css_prop: StyleProperty, default: Brush) -> Brush {
        let doc = &self.layer_list.layout_tree.render_tree.doc;
        match doc.get_style(node_id, css_prop) {
            Some(StyleValue::Color(css_color)) => Brush::solid(convert_css_color(&css_color)),
            _ => default,
        }
    }

    fn get_parent_brush(&self, node_id: NodeId, css_prop: StyleProperty, default: Brush) -> Brush {
        let doc = &self.layer_list.layout_tree.render_tree.doc;
        match doc.parent(node_id) {
            Some(parent_id) => self.get_brush(parent_id, css_prop, default),
            None => default,
        }
    }

    /// Generates the wireframe commands for the given layout element
    fn generate_wireframe_commands(&self, layout_element: &LayoutElementNode) -> Vec<PaintCommand> {
        let mut commands = Vec::new();

        let border = Border::new(
            1.0,
            BorderStyle::Solid,
            [
                Brush::Solid(Color::RED),
                Brush::Solid(Color::RED),
                Brush::Solid(Color::RED),
                Brush::Solid(Color::RED),
            ],
        );
        let r = Rectangle::new(layout_element.box_model.border_box).with_border(border);
        commands.push(PaintCommand::rectangle(r));

        commands
    }

    /// Generates the boxmodel commands for the given layout element
    fn generate_boxmodel_commands(&self, layout_element: &LayoutElementNode) -> Vec<PaintCommand> {
        let mut commands = Vec::new();

        let brush = Brush::Solid(Color::YELLOW);
        let r = Rectangle::new(layout_element.box_model.margin_box).with_background(brush);
        commands.push(PaintCommand::rectangle(r));

        let brush = Brush::Solid(Color::GREEN);
        let r = Rectangle::new(layout_element.box_model.padding_box).with_background(brush);
        commands.push(PaintCommand::rectangle(r));

        let brush = Brush::Solid(Color::CYAN);
        let r = Rectangle::new(layout_element.box_model.content_box).with_background(brush);
        commands.push(PaintCommand::rectangle(r));

        commands
    }

    /// Generates the paint commands for the given layout element
    fn generate_element_commands(&self, layout_element: &LayoutElementNode, dom_node_id: NodeId) -> Vec<PaintCommand> {
        let mut commands = Vec::new();

        match &layout_element.context {
            ElementContext::Text(ctx) => {
                let brush = self.get_parent_brush(dom_node_id, StyleProperty::Color, Brush::solid(Color::BLACK));

                // let r = layout_element.box_model.content_box().shift(ctx.text_offset);
                let r = layout_element.box_model.content_box;
                // let brush = Brush::solid(Color::from_rgb8(130, 130, 130));
                let t = Text::new(r, &ctx.text, &ctx.font_info, brush);
                commands.push(PaintCommand::text(t));

                // let border = Border::new(1.0, BorderStyle::Solid, Brush::Solid(Color::RED));
                // let r = Rectangle::new(layout_element.box_model.border_box()).with_border(border);
                // let r = Rectangle::new(layout_element.box_model.border_box()); // .with_border(border);
                // commands.push(PaintCommand::rectangle(r));
            }
            ElementContext::Svg(svg_ctx) => {
                // let binding = get_svg_store();
                // let svg_store = binding.read().expect("Failed to get svg store");
                // let svg = svg_store.get(svg_ctx.svg_id).unwrap();

                let brush = Brush::solid(Color::from_rgb8(130, 130, 130));
                let r = Rectangle::new(layout_element.box_model.border_box).with_background(brush);
                commands.push(PaintCommand::svg(svg_ctx.media_id, r));
            }
            ElementContext::Image(image_ctx) => {
                let brush = Brush::image(image_ctx.media_id);
                let r = Rectangle::new(layout_element.box_model.border_box).with_background(brush);
                commands.push(PaintCommand::rectangle(r));
            }
            ElementContext::None => {
                // Paint a normal element. This function will most likely be much more complex as it is now, because we need to
                // deal with other elements line input fields, buttons, etc. But for now, we just paint a rectangle with (rounded) borders and
                // brush.

                let doc = &self.layer_list.layout_tree.render_tree.doc;
                let brush = self.get_brush(
                    dom_node_id,
                    StyleProperty::BackgroundColor,
                    Brush::solid(Color::TRANSPARENT),
                );
                let mut r = Rectangle::new(layout_element.box_model.border_box).with_background(brush);

                // Get border
                let border_top_width = doc.get_style_f32(dom_node_id, StyleProperty::BorderTopWidth);
                let border_right_width = doc.get_style_f32(dom_node_id, StyleProperty::BorderRightWidth);
                let border_bottom_width = doc.get_style_f32(dom_node_id, StyleProperty::BorderBottomWidth);
                let border_left_width = doc.get_style_f32(dom_node_id, StyleProperty::BorderLeftWidth);

                if border_top_width != 0.0
                    || border_right_width != 0.0
                    || border_bottom_width != 0.0
                    || border_left_width != 0.0
                {
                    let border_top_color =
                        self.get_brush(dom_node_id, StyleProperty::BorderTopColor, Brush::solid(Color::BLACK));
                    let border_right_color =
                        self.get_brush(dom_node_id, StyleProperty::BorderRightColor, Brush::solid(Color::BLACK));
                    let border_bottom_color = self.get_brush(
                        dom_node_id,
                        StyleProperty::BorderBottomColor,
                        Brush::solid(Color::BLACK),
                    );
                    let border_left_color =
                        self.get_brush(dom_node_id, StyleProperty::BorderLeftColor, Brush::solid(Color::BLACK));

                    let border = Border::new(
                        border_top_width,
                        BorderStyle::Solid,
                        [
                            border_top_color,
                            border_right_color,
                            border_bottom_color,
                            border_left_color,
                        ],
                    );
                    r = r.with_border(border);
                }

                // Get radius
                let radius_bottom_left = doc.get_style_f32(dom_node_id, StyleProperty::BorderBottomLeftRadius);
                let radius_bottom_right = doc.get_style_f32(dom_node_id, StyleProperty::BorderBottomRightRadius);
                let radius_top_left = doc.get_style_f32(dom_node_id, StyleProperty::BorderTopLeftRadius);
                let radius_top_right = doc.get_style_f32(dom_node_id, StyleProperty::BorderTopRightRadius);

                if (radius_bottom_left != 0.0
                    || radius_bottom_right != 0.0
                    || radius_top_left != 0.0
                    || radius_top_right != 0.0)
                {
                    r = r.with_radius_tlrb(
                        Radius::new(radius_top_left as f64),
                        Radius::new(radius_top_right as f64),
                        Radius::new(radius_bottom_right as f64),
                        Radius::new(radius_bottom_left as f64),
                    );
                }

                commands.push(PaintCommand::rectangle(r));
            }
        }

        commands
    }
}

/// Converts a css style color to a paint command color
fn convert_css_color(css_color: &StyleColor) -> Color {
    log::debug!("Converting css color: {:?}", css_color);
    match css_color {
        StyleColor::Named(name) => Color::from_css(name.as_str()),
        StyleColor::Rgb(r, g, b) => Color::from_rgb8(*r, *g, *b),
        StyleColor::Rgba(r, g, b, a) => Color::from_rgba8(*r, *g, *b, (*a * 255.0) as u8),
    }
}
