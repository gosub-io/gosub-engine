pub mod commands;

use crate::common::browser_state::WireframeState;
use crate::common::document::node::{Node, NodeType};
use crate::common::document::style::{Color as StyleColor, StyleProperty, StyleValue};
use crate::layering::layer::LayerList;
use crate::layouter::{ElementContext, LayoutElementId, LayoutElementNode};
use crate::painter::commands::border::{Border, BorderStyle};
use crate::painter::commands::brush::Brush;
use crate::painter::commands::color::Color;
use crate::painter::commands::rectangle::{Radius, Rectangle};
use crate::painter::commands::text::Text;
use crate::painter::commands::PaintCommand;
use crate::tiler::TiledLayoutElement;
use std::sync::Arc;

/// Options that control how the painter renders elements (debug modes, etc.)
pub struct PaintOptions {
    /// Controls wireframe rendering mode
    pub wireframed: WireframeState,
    /// Highlight the hovered element's box model
    pub debug_hover: bool,
    /// The currently hovered element (for box-model debug overlay)
    pub current_hovered_element: Option<LayoutElementId>,
}

impl Default for PaintOptions {
    fn default() -> Self {
        Self {
            wireframed: WireframeState::None,
            debug_hover: false,
            current_hovered_element: None,
        }
    }
}

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

    /// Generate paint commands for the given tiled element.
    pub fn paint(&self, element: &TiledLayoutElement, options: &PaintOptions) -> Vec<PaintCommand> {
        let mut commands = Vec::new();

        let Some(layout_element) = self.layer_list.layout_tree.get_node_by_id(element.id) else {
            return Vec::new();
        };
        let Some(dom_node) = self
            .layer_list
            .layout_tree
            .render_tree
            .doc
            .get_node_by_id(layout_element.dom_node_id)
        else {
            return Vec::new();
        };

        // Paint boxmodel for the hovered element if needed
        if options.debug_hover
            && options.current_hovered_element.is_some()
            && options.current_hovered_element.unwrap() == layout_element.id
        {
            commands.extend(self.generate_boxmodel_commands(layout_element));
        }

        match options.wireframed {
            WireframeState::Only => {
                commands.extend(self.generate_wireframe_commands(layout_element));
            }
            WireframeState::Both => {
                commands.extend(self.generate_element_commands(layout_element, dom_node));
                commands.extend(self.generate_wireframe_commands(layout_element));
            }
            WireframeState::None => {
                commands.extend(self.generate_element_commands(layout_element, dom_node));
            }
        }

        commands
    }

    // Returns a brush for the color found in the given dom node
    fn get_brush(&self, node: &Node, css_prop: StyleProperty, default: Brush) -> Brush {
        let NodeType::Element(element_data) = &node.node_type else {
            return default;
        };
        element_data
            .get_style(css_prop)
            .map_or(default.clone(), |value| match value {
                StyleValue::Color(css_color) => Brush::solid(convert_css_color(css_color)),
                _ => default.clone(),
            })
    }

    // Returns a brush for the color found in the PARENT of the given dom node
    fn get_parent_brush(&self, node: &Node, css_prop: StyleProperty, default: Brush) -> Brush {
        let parent = match &node.parent_id {
            Some(parent_id) => self.layer_list.layout_tree.render_tree.doc.get_node_by_id(*parent_id),
            None => return default,
        };
        match parent {
            Some(p) => self.get_brush(p, css_prop, default),
            None => default,
        }
    }

    fn generate_wireframe_commands(&self, layout_element: &LayoutElementNode) -> Vec<PaintCommand> {
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
        vec![PaintCommand::rectangle(r)]
    }

    fn generate_boxmodel_commands(&self, layout_element: &LayoutElementNode) -> Vec<PaintCommand> {
        vec![
            PaintCommand::rectangle(
                Rectangle::new(layout_element.box_model.margin_box).with_background(Brush::Solid(Color::YELLOW)),
            ),
            PaintCommand::rectangle(
                Rectangle::new(layout_element.box_model.padding_box).with_background(Brush::Solid(Color::GREEN)),
            ),
            PaintCommand::rectangle(
                Rectangle::new(layout_element.box_model.content_box).with_background(Brush::Solid(Color::CYAN)),
            ),
        ]
    }

    fn generate_element_commands(&self, layout_element: &LayoutElementNode, dom_node: &Node) -> Vec<PaintCommand> {
        let mut commands = Vec::new();

        match &layout_element.context {
            ElementContext::Text(ctx) => {
                let brush = self.get_parent_brush(dom_node, StyleProperty::Color, Brush::solid(Color::BLACK));
                let r = layout_element.box_model.padding_box;
                let t = Text::new(r, &ctx.text, &ctx.font_info, brush);
                commands.push(PaintCommand::text(t));
            }
            ElementContext::Svg(svg_ctx) => {
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
                let brush = self.get_brush(
                    dom_node,
                    StyleProperty::BackgroundColor,
                    Brush::solid(Color::TRANSPARENT),
                );
                let mut r = Rectangle::new(layout_element.box_model.border_box).with_background(brush);

                let border_top = dom_node.get_style_f32(StyleProperty::BorderTopWidth);
                let border_right = dom_node.get_style_f32(StyleProperty::BorderRightWidth);
                let border_bottom = dom_node.get_style_f32(StyleProperty::BorderBottomWidth);
                let border_left = dom_node.get_style_f32(StyleProperty::BorderLeftWidth);

                if border_top != 0.0 || border_right != 0.0 || border_bottom != 0.0 || border_left != 0.0 {
                    let border = Border::new(
                        border_top,
                        BorderStyle::Solid,
                        [
                            self.get_brush(dom_node, StyleProperty::BorderTopColor, Brush::solid(Color::BLACK)),
                            self.get_brush(dom_node, StyleProperty::BorderRightColor, Brush::solid(Color::BLACK)),
                            self.get_brush(dom_node, StyleProperty::BorderBottomColor, Brush::solid(Color::BLACK)),
                            self.get_brush(dom_node, StyleProperty::BorderLeftColor, Brush::solid(Color::BLACK)),
                        ],
                    );
                    r = r.with_border(border);
                }

                let rl = dom_node.get_style_f32(StyleProperty::BorderBottomLeftRadius);
                let rr = dom_node.get_style_f32(StyleProperty::BorderBottomRightRadius);
                let rtl = dom_node.get_style_f32(StyleProperty::BorderTopLeftRadius);
                let rtr = dom_node.get_style_f32(StyleProperty::BorderTopRightRadius);

                if rl != 0.0 || rr != 0.0 || rtl != 0.0 || rtr != 0.0 {
                    r = r.with_radius_tlrb(
                        Radius::new(rtl as f64),
                        Radius::new(rtr as f64),
                        Radius::new(rr as f64),
                        Radius::new(rl as f64),
                    );
                }

                commands.push(PaintCommand::rectangle(r));
            }
        }

        commands
    }
}

fn convert_css_color(css_color: &StyleColor) -> Color {
    match css_color {
        StyleColor::Named(name) => Color::from_css(name.as_str()),
        StyleColor::Rgb(r, g, b) => Color::from_rgb8(*r, *g, *b),
        StyleColor::Rgba(r, g, b, a) => Color::from_rgba8(*r, *g, *b, (*a * 255.0) as u8),
    }
}
