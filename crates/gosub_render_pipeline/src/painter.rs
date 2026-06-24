pub mod commands;

use crate::common::browser_state::{BrowserState, WireframeState};
use crate::common::document::node::NodeId;
use crate::common::document::style::{BorderStyle as CssBorderStyle, Display, StyleProperty, Value};
use crate::common::media::MediaStore;
use crate::layering::layer::LayerList;
use crate::layouter::{BackgroundMedia, ElementContext, LayoutElementId, LayoutElementNode};
use crate::painter::commands::border::{Border, BorderStyle};
use crate::painter::commands::brush::Brush;
use crate::painter::commands::color::Color;
use crate::painter::commands::rectangle::{Radius, Rectangle};
use crate::painter::commands::text::Text;
use crate::painter::commands::PaintCommand;
use crate::tiler::TiledLayoutElement;
use std::sync::Arc;

/// A whole-viewport paint command list plus the media store needed to resolve image/SVG ids.
///
/// Produced by the engine's GPU-scene path (one ordered list for the entire page, in z-order)
/// and consumed by a GPU backend's `render`, which translates the commands into its native
/// scene. This replaces the tile/rasterize/composite stages for GPU backends.
pub struct PaintScene {
    /// All paint commands for the page, in paint order (bottom layer first).
    pub commands: Vec<PaintCommand>,
    /// Shared media store; commands reference images/SVGs by id and resolve them here.
    pub media_store: Arc<MediaStore>,
    /// Full laid-out page height in CSS pixels (for scroll clamping on the host).
    pub page_height: f64,
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

    // Generate paint commands for the given tile
    pub fn paint(&self, element: &TiledLayoutElement, state: &BrowserState) -> Vec<PaintCommand> {
        self.paint_element(element.id, state)
    }

    /// Paint every element in the layer list into a single flat command list, in z-order
    /// (`layer_ids` order) then paint order (`layer.elements` order). Used by GPU-scene
    /// backends that render the whole viewport in one pass instead of per tile. The result
    /// matches the z-ordering the tiler produces.
    pub fn paint_all(&self, state: &BrowserState) -> Vec<PaintCommand> {
        let mut out = Vec::new();
        let layer_ids = self.layer_list.layer_ids.read();
        let layers = self.layer_list.layers.read();
        for layer_id in layer_ids.iter() {
            let Some(layer) = layers.get(layer_id) else {
                continue;
            };
            for &element_id in &layer.elements {
                out.extend(self.paint_element(element_id, state));
            }
        }
        out
    }

    /// Generate paint commands for a single layout element.
    pub fn paint_element(&self, element_id: LayoutElementId, state: &BrowserState) -> Vec<PaintCommand> {
        let mut commands = Vec::new();

        let Some(layout_element) = self.layer_list.layout_tree.get_node_by_id(element_id) else {
            return Vec::new();
        };
        let dom_node_id = layout_element.dom_node_id;

        // Paint boxmodel for the hovered element if needed
        if state.debug_hover && state.current_hovered_element == Some(layout_element.id) {
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

        if state.debug_table_cells {
            commands.extend(self.generate_table_debug_commands(layout_element, dom_node_id));
        }

        commands
    }

    fn get_brush(&self, node_id: NodeId, css_prop: &StyleProperty, default: Brush) -> Brush {
        let doc = &self.layer_list.layout_tree.render_tree.doc;
        let brush = match doc.get_style(node_id, css_prop) {
            Value::Color(r, g, b, a) => Brush::solid(Color::from_rgba8(r, g, b, a)),
            _ => default,
        };
        self.apply_opacity(node_id, brush)
    }

    /// Scales a brush's alpha by the element's `opacity`. This is a per-element approximation
    /// (it does not group-composite the element with its descendants), which is exact for
    /// leaf boxes such as a 1px `::before` divider and a reasonable approximation otherwise.
    fn apply_opacity(&self, node_id: NodeId, brush: Brush) -> Brush {
        // Elements promoted into an opacity compositing group are faded as a whole layer at
        // composite time; applying opacity per-element here too would darken them twice.
        if self.layer_list.is_opacity_grouped(node_id) {
            return brush;
        }

        let doc = &self.layer_list.layout_tree.render_tree.doc;
        let opacity = match doc.get_style(node_id, &StyleProperty::Opacity) {
            Value::Number(n) | Value::Unit(n, _) => n,
            _ => 1.0,
        };
        if opacity >= 1.0 {
            return brush;
        }
        let op = opacity.clamp(0.0, 1.0);
        match brush {
            Brush::Solid(c) => Brush::Solid(Color::from_rgba(c.r(), c.g(), c.b(), c.a() * op)),
            // Gradient/image opacity (true group compositing) is not yet modelled.
            other => other,
        }
    }

    /// The fill for an element's background box: a `linear-gradient(...)` if present,
    /// otherwise the solid `background-color` (transparent when unset).
    fn background_brush(&self, node_id: NodeId) -> Brush {
        let doc = &self.layer_list.layout_tree.render_tree.doc;
        if let Some(gradient) = doc.background_gradient(node_id) {
            return Brush::gradient(gradient);
        }
        self.get_brush(
            node_id,
            &StyleProperty::BackgroundColor,
            Brush::solid(Color::TRANSPARENT),
        )
    }

    fn get_parent_brush(&self, node_id: NodeId, css_prop: &StyleProperty, default: Brush) -> Brush {
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

    /// Overlays a colored 1px border for table-related display roles (debug only).
    fn generate_table_debug_commands(
        &self,
        layout_element: &LayoutElementNode,
        dom_node_id: NodeId,
    ) -> Vec<PaintCommand> {
        let doc = &self.layer_list.layout_tree.render_tree.doc;
        let color = match doc.get_own_style(dom_node_id, &StyleProperty::Display) {
            Some(Value::Display(Display::Table)) => Color::from_rgb8(255, 0, 0),
            Some(Value::Display(Display::TableCell)) => Color::from_rgb8(0, 180, 0),
            Some(Value::Display(Display::TableRow)) => Color::from_rgb8(0, 0, 255),
            Some(Value::Display(Display::TableRowGroup))
            | Some(Value::Display(Display::TableHeaderGroup))
            | Some(Value::Display(Display::TableFooterGroup)) => Color::from_rgb8(160, 0, 200),
            Some(Value::Display(Display::TableCaption)) => Color::from_rgb8(255, 140, 0),
            _ => return Vec::new(),
        };
        let border = Border::new(
            1.0,
            BorderStyle::Solid,
            [
                Brush::Solid(color.clone()),
                Brush::Solid(color.clone()),
                Brush::Solid(color.clone()),
                Brush::Solid(color),
            ],
        );
        let r = Rectangle::new(layout_element.box_model.border_box).with_border(border);
        vec![PaintCommand::rectangle(r)]
    }

    /// Paint commands for an element's CSS `background-image`, filling the border box.
    /// Raster images are blitted via `Brush::image`; SVGs go through the SVG paint path
    /// (e.g. HN's `triangle.svg` votearrow), which `Brush::image` cannot render.
    fn background_media_commands(
        &self,
        bg: BackgroundMedia,
        layout_element: &LayoutElementNode,
        dom_node_id: NodeId,
    ) -> Vec<PaintCommand> {
        let border_box = layout_element.box_model.border_box;
        match bg {
            BackgroundMedia::Image(media_id) => {
                let brush = Brush::image(media_id);
                let r = Rectangle::new(border_box).with_background(brush);
                let r = self.decorate_with_border_and_radius(dom_node_id, r);
                vec![PaintCommand::rectangle(r)]
            }
            BackgroundMedia::Svg(media_id) => vec![PaintCommand::svg(media_id, Rectangle::new(border_box))],
        }
    }

    /// Generates the paint commands for the given layout element
    fn generate_element_commands(&self, layout_element: &LayoutElementNode, dom_node_id: NodeId) -> Vec<PaintCommand> {
        let mut commands = Vec::new();

        // CSS background-image. For plain block elements (`None` context) it is painted just
        // after the background-color in that branch below (correct CSS layering). For
        // replaced/text content we paint it first so the element's own content stays on top.
        let bg_media = layout_element.background_media;
        if let Some(bg) = bg_media {
            if !matches!(layout_element.context, ElementContext::None) {
                commands.extend(self.background_media_commands(bg, layout_element, dom_node_id));
            }
        }

        match &layout_element.context {
            ElementContext::Text(ctx) => {
                let brush = self.get_parent_brush(dom_node_id, &StyleProperty::Color, Brush::solid(Color::BLACK));
                let brush = self.apply_opacity(dom_node_id, brush);

                let r = layout_element.box_model.content_box;
                let avail_w = if ctx.available_width > 0.0 {
                    ctx.available_width
                } else {
                    1_000_000_000.0
                };
                let t = Text::new(r, &ctx.text, &ctx.font_info, brush, avail_w);
                commands.push(PaintCommand::text(t));
            }
            ElementContext::Svg(svg_ctx) => {
                let border_box = layout_element.box_model.border_box;
                commands.push(PaintCommand::svg(svg_ctx.media_id, Rectangle::new(border_box)));
                // The SVG painter doesn't draw the element's CSS border/radius, so emit it as a
                // separate border-only rectangle painted on top of the icon (e.g. the HN logo's
                // `border:1px white solid`).
                if self.has_border(dom_node_id) {
                    let r = self.decorate_with_border_and_radius(dom_node_id, Rectangle::new(border_box));
                    commands.push(PaintCommand::rectangle(r));
                }
            }
            ElementContext::Image(image_ctx) => {
                let brush = Brush::image(image_ctx.media_id);
                let r = Rectangle::new(layout_element.box_model.border_box).with_background(brush);
                let r = self.decorate_with_border_and_radius(dom_node_id, r);
                commands.push(PaintCommand::rectangle(r));
            }
            ElementContext::None => {
                let brush = self.background_brush(dom_node_id);
                let r = Rectangle::new(layout_element.box_model.border_box).with_background(brush);
                let r = self.decorate_with_border_and_radius(dom_node_id, r);
                commands.push(PaintCommand::rectangle(r));

                // background-image paints on top of the background-color.
                if let Some(bg) = bg_media {
                    commands.extend(self.background_media_commands(bg, layout_element, dom_node_id));
                }
            }
        }

        commands
    }

    /// Returns true when the element has a non-zero border on any edge.
    fn has_border(&self, dom_node_id: NodeId) -> bool {
        let doc = &self.layer_list.layout_tree.render_tree.doc;
        doc.get_style_f32(dom_node_id, &StyleProperty::BorderTopWidth) != 0.0
            || doc.get_style_f32(dom_node_id, &StyleProperty::BorderRightWidth) != 0.0
            || doc.get_style_f32(dom_node_id, &StyleProperty::BorderBottomWidth) != 0.0
            || doc.get_style_f32(dom_node_id, &StyleProperty::BorderLeftWidth) != 0.0
    }

    /// Apply the element's computed CSS border and border-radius to `r`. Shared by block,
    /// image and SVG elements so replaced elements (`<img>`) get their borders too.
    fn decorate_with_border_and_radius(&self, dom_node_id: NodeId, mut r: Rectangle) -> Rectangle {
        let doc = &self.layer_list.layout_tree.render_tree.doc;

        let border_top_width = doc.get_style_f32(dom_node_id, &StyleProperty::BorderTopWidth);
        let border_right_width = doc.get_style_f32(dom_node_id, &StyleProperty::BorderRightWidth);
        let border_bottom_width = doc.get_style_f32(dom_node_id, &StyleProperty::BorderBottomWidth);
        let border_left_width = doc.get_style_f32(dom_node_id, &StyleProperty::BorderLeftWidth);

        if border_top_width != 0.0
            || border_right_width != 0.0
            || border_bottom_width != 0.0
            || border_left_width != 0.0
        {
            let border_top_color =
                self.get_brush(dom_node_id, &StyleProperty::BorderTopColor, Brush::solid(Color::BLACK));
            let border_right_color = self.get_brush(
                dom_node_id,
                &StyleProperty::BorderRightColor,
                Brush::solid(Color::BLACK),
            );
            let border_bottom_color = self.get_brush(
                dom_node_id,
                &StyleProperty::BorderBottomColor,
                Brush::solid(Color::BLACK),
            );
            let border_left_color =
                self.get_brush(dom_node_id, &StyleProperty::BorderLeftColor, Brush::solid(Color::BLACK));

            let side_style = |prop: &StyleProperty| match doc.get_style(dom_node_id, prop) {
                Value::BorderStyle(s) => css_border_style_to_paint(&s),
                _ => BorderStyle::Solid,
            };
            let border = Border::new_per_side(
                [
                    border_top_width,
                    border_right_width,
                    border_bottom_width,
                    border_left_width,
                ],
                [
                    side_style(&StyleProperty::BorderTopStyle),
                    side_style(&StyleProperty::BorderRightStyle),
                    side_style(&StyleProperty::BorderBottomStyle),
                    side_style(&StyleProperty::BorderLeftStyle),
                ],
                [
                    border_top_color,
                    border_right_color,
                    border_bottom_color,
                    border_left_color,
                ],
            );
            r = r.with_border(border);
        }

        let radius_bottom_left = doc.get_style_f32(dom_node_id, &StyleProperty::BorderBottomLeftRadius);
        let radius_bottom_right = doc.get_style_f32(dom_node_id, &StyleProperty::BorderBottomRightRadius);
        let radius_top_left = doc.get_style_f32(dom_node_id, &StyleProperty::BorderTopLeftRadius);
        let radius_top_right = doc.get_style_f32(dom_node_id, &StyleProperty::BorderTopRightRadius);

        if radius_bottom_left != 0.0 || radius_bottom_right != 0.0 || radius_top_left != 0.0 || radius_top_right != 0.0
        {
            r = r.with_radius_tlrb(
                Radius::new(radius_top_left as f64),
                Radius::new(radius_top_right as f64),
                Radius::new(radius_bottom_right as f64),
                Radius::new(radius_bottom_left as f64),
            );
        }

        r
    }
}

fn css_border_style_to_paint(s: &CssBorderStyle) -> BorderStyle {
    match s {
        CssBorderStyle::Solid => BorderStyle::Solid,
        CssBorderStyle::Dashed => BorderStyle::Dashed,
        CssBorderStyle::Dotted => BorderStyle::Dotted,
        CssBorderStyle::Double => BorderStyle::Double,
        CssBorderStyle::Groove => BorderStyle::Groove,
        CssBorderStyle::Ridge => BorderStyle::Ridge,
        CssBorderStyle::Inset => BorderStyle::Inset,
        CssBorderStyle::Outset => BorderStyle::Outset,
        CssBorderStyle::Hidden => BorderStyle::Hidden,
        CssBorderStyle::None => BorderStyle::None,
    }
}
