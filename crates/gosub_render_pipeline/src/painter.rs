pub mod commands;

use std::ops::AddAssign;
use std::sync::Arc;
use rand::Rng;
use gosub_interface::config::HasDocument;
use gosub_interface::node::Node;
use crate::common::render_state::{RenderState, WireframeState};
use crate::common::style::{StyleProperty, StyleValue, Color as StyleColor};
use crate::layering::layer::LayerList;
use crate::layouter::{ElementContext, LayoutElementNode};
use crate::painter::commands::brush::Brush;
use crate::painter::commands::color::Color;
use crate::painter::commands::rectangle::{Radius, Rectangle};
use crate::painter::commands::PaintCommand;
use crate::common::get_media_store;
use crate::common::media::{Media, MediaType};
use crate::painter::commands::border::{Border, BorderStyle};
use crate::painter::commands::text::Text;
use crate::tiler::{Tile, TiledLayoutElement};
use crate::with_render_state;

/// Painter works with the layout tree and generates paint commands for the renderer. It does not
/// generate a new data structure as output, but will update the existing layout elements with
/// paint commands.
pub struct Painter<C: HasDocument> {
    layer_list: Arc<LayerList<C>>,
}

impl<C: HasDocument> Painter<C> {
    pub fn new(layer_list: Arc<LayerList<C>>) -> Painter<C> {
        Painter {
            layer_list
        }
    }

    // Generate paint commands for the given tile
    pub fn paint(&self, element: &TiledLayoutElement) -> Vec<PaintCommand> {
        let mut commands = Vec::new();

        let Some(layout_element) = self.layer_list.layout_tree.get_node_by_id(element.id) else {
            return Vec::new();
        };
        let Some(dom_node) = self.layer_list.layout_tree.render_tree.doc.get_node_by_id(layout_element.dom_node_id) else {
            return Vec::new();
        };

        with_render_state!(C, state => {
            // Paint boxmodel for the hovered element if needed
            if state.debug_hover && state.current_hovered_element.is_some() && state.current_hovered_element.unwrap() == layout_element.id {
                commands.extend(self.generate_boxmodel_commands(&layout_element));
            }

            match state.wireframed {
                WireframeState::Only => {
                    // Paint only the wireframe of the element
                    commands.extend(self.generate_wireframe_commands(&layout_element));
                }
                WireframeState::Both => {
                    // Paint both the wireframe and element
                    commands.extend(self.generate_element_commands(&layout_element, &dom_node));
                    commands.extend(self.generate_wireframe_commands(&layout_element));
                }
                WireframeState::None => {
                    // Paint only the element. No debug/developer wireframe is needed.
                    commands.extend(self.generate_element_commands(&layout_element, &dom_node));
                }
            }

            commands
        });
    }

    // Returns a brush for the color found in the given dom node
    fn get_brush(&self, node: &C::Node, css_prop: StyleProperty, default: Brush) -> Brush {
        let Some(element_data) = node.get_element_data() else {
            log::warn!("Failed to get element data for node: {:?}", node.id());
            return default;
        };

        element_data.get_style(css_prop).map_or(default.clone(), |value| {
            match value {
                StyleValue::Color(css_color) => Brush::solid(convert_css_color(css_color)),
                _ => {
                    log::warn!("Failed to get brush for node: {:?}", node.id());
                    default.clone()
                }
            }
        })
    }

    // Returns a brush for the color found in the PARENT of the given dom node
    fn get_parent_brush(&self, node: &C::Node, css_prop: StyleProperty, default: Brush) -> Brush {
        let parent = match &node.parent_id {
            Some(parent_id) => self.layer_list.layout_tree.render_tree.doc.get_node_by_id(*parent_id).expect("Failed to get parent node"),
            None => {
                log::warn!("Failed to get parent brush for node: {:?}", node.node_id);
                return default
            },
        };

        self.get_brush(parent, css_prop, default)
    }

    /// Generates the wireframe commands for the given layout element
    fn generate_wireframe_commands(&self, layout_element: &LayoutElementNode) -> Vec<PaintCommand> {
        let mut commands = Vec::new();

        let border = Border::new(1.0, BorderStyle::Solid, [
            Brush::Solid(Color::RED),
            Brush::Solid(Color::RED),
            Brush::Solid(Color::RED),
            Brush::Solid(Color::RED),
        ]);
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
    fn generate_element_commands(&self, layout_element: &LayoutElementNode, dom_node: &C::Node) -> Vec<PaintCommand> {
        let mut commands = Vec::new();

        match &layout_element.context {
            ElementContext::Text(ctx) => {
                let brush = self.get_parent_brush(dom_node, StyleProperty::Color, Brush::solid(Color::BLACK));

                // let r = layout_element.box_model.content_box().shift(ctx.text_offset);
                let r = layout_element.box_model.padding_box;
                // let brush = Brush::solid(Color::from_rgb8(130, 130, 130));
                let t = Text::new(
                    r,
                    &ctx.text,
                    &ctx.font_info,
                    brush,
                );
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

                let brush = self.get_brush(dom_node, StyleProperty::BackgroundColor, Brush::solid(Color::TRANSPARENT));
                let mut r = Rectangle::new(layout_element.box_model.border_box).with_background(brush);

                // Get border
                let border_top_width = dom_node.get_style_f32(StyleProperty::BorderTopWidth);
                let border_right_width = dom_node.get_style_f32(StyleProperty::BorderRightWidth);
                let border_bottom_width = dom_node.get_style_f32(StyleProperty::BorderBottomWidth);
                let border_left_width = dom_node.get_style_f32(StyleProperty::BorderLeftWidth);

                if (border_top_width != 0.0 || border_right_width != 0.0 || border_bottom_width != 0.0 || border_left_width != 0.0) {
                    let border_top_color = self.get_brush(dom_node, StyleProperty::BorderTopColor, Brush::solid(Color::BLACK));
                    let border_right_color = self.get_brush(dom_node, StyleProperty::BorderRightColor, Brush::solid(Color::BLACK));
                    let border_bottom_color = self.get_brush(dom_node, StyleProperty::BorderBottomColor, Brush::solid(Color::BLACK));
                    let border_left_color = self.get_brush(dom_node, StyleProperty::BorderLeftColor, Brush::solid(Color::BLACK));

                    let border = Border::new(
                        border_top_width,
                        BorderStyle::Solid,
                        [
                            border_top_color,
                            border_right_color,
                            border_bottom_color,
                            border_left_color,
                        ]
                    );
                    r = r.with_border(border);
                }

                // Get radius
                let radius_bottom_left = dom_node.get_style_f32(StyleProperty::BorderBottomLeftRadius);
                let radius_bottom_right = dom_node.get_style_f32(StyleProperty::BorderBottomRightRadius);
                let radius_top_left = dom_node.get_style_f32(StyleProperty::BorderTopLeftRadius);
                let radius_top_right = dom_node.get_style_f32(StyleProperty::BorderTopRightRadius);

                if (radius_bottom_left != 0.0 || radius_bottom_right != 0.0 || radius_top_left != 0.0 || radius_top_right != 0.0) {
                    r = r.with_radius_tlrb(
                        Radius::new(radius_top_left as f64),
                        Radius::new(radius_top_right as f64),
                        Radius::new(radius_bottom_right as f64),
                        Radius::new(radius_bottom_left as f64)
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
    log::info!("Converting css color: {:?}", css_color);
    match css_color {
        StyleColor::Named(name) => Color::from_css(name.as_str()),
        StyleColor::Rgb(r, g, b) => Color::from_rgb8(*r, *g, *b),
        StyleColor::Rgba(r, g, b, a) => Color::from_rgba8(*r, *g, *b, (*a * 255.0) as u8),
    }
}

