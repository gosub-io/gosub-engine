pub mod commands;

use crate::common::browser_state::{BrowserState, WireframeState};
use crate::common::document::node::NodeId;
use crate::common::document::style::{lookup, BorderStyle as CssBorderStyle, Display, StyleProperty, Value};
use crate::common::font::{FontAlignment, FontInfo};
use crate::common::media::MediaStore;
use crate::layering::layer::LayerList;
use crate::layouter::{BackgroundMedia, ElementContext, LayoutElementId, LayoutElementNode};
use crate::painter::commands::border::{Border, BorderStyle};
use crate::painter::commands::brush::Brush;
use crate::painter::commands::color::Color;
use crate::painter::commands::gradient::Gradient;
use crate::painter::commands::rectangle::{BlendMode, Radius, Rectangle};
use crate::painter::commands::text::Text;
use crate::painter::commands::PaintCommand;
use crate::render::backend::TileAnchor;
use crate::tiler::TiledLayoutElement;
use gosub_interface::font::FontStyle;
use gosub_interface::font_system::{FontStretch, FontSystem, FontWeight, ShapedText, TextAlign, TextStyle};
use parking_lot::Mutex;
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

/// The neutral [`TextStyle`] for a text paint command — the same mapping the layouter's measure
/// path uses, plus the wrap/alignment container, so shaping reproduces the measured box.
///
/// Wrap limit: Start-aligned text wraps within the container width the layouter used, so the
/// shaped line breaks reproduce the measured ones (text fragments can carry whole multi-line
/// paragraphs). Center/End/Justify text instead uses the fragment's own box as its alignment
/// container — glyphs shifted outside the fragment rect would land in tiles that never repaint
/// the command.
fn paint_text_style(font_info: &FontInfo, rect_width: f64, available_width: f64) -> TextStyle {
    let align = match font_info.alignment {
        FontAlignment::Start => TextAlign::Start,
        FontAlignment::Center => TextAlign::Center,
        FontAlignment::End => TextAlign::End,
        FontAlignment::Justify => TextAlign::Justify,
    };
    let max_width = match align {
        TextAlign::Start => available_width.max(rect_width).max(1.0) as f32,
        _ => rect_width.max(1.0) as f32,
    };
    TextStyle {
        family: font_info.family.clone(),
        size: font_info.size as f32,
        weight: FontWeight(font_info.weight.clamp(1, 1000) as u16),
        style: if font_info.slant != 0 {
            FontStyle::Italic
        } else {
            FontStyle::Normal
        },
        stretch: FontStretch::NORMAL,
        line_height: Some(font_info.line_height as f32),
        letter_spacing: font_info.letter_spacing as f32,
        max_width: Some(max_width),
        align,
        // Paint commands are in CSS pixels; DPI scaling is applied later in the pipeline.
        display_scale: 1.0,
    }
}

/// Painter works with the layout tree and generates paint commands for the renderer. It does not
/// generate a new data structure as output, but will update the existing layout elements with
/// paint commands.
pub struct Painter {
    layer_list: Arc<LayerList>,
    /// The engine's shared font system — the same instance the layouter measured with. Text is
    /// shaped here, once, at command-build time; rasterizers paint the shaped runs. `None` (no
    /// rasterizer font system, e.g. the null backend) produces commands with empty glyph runs,
    /// which only engine-native text rasterizers can still draw.
    font_system: Option<Arc<Mutex<dyn FontSystem>>>,
}

impl Painter {
    pub fn new(layer_list: Arc<LayerList>, font_system: Option<Arc<Mutex<dyn FontSystem>>>) -> Painter {
        Painter {
            layer_list,
            font_system,
        }
    }

    /// Shape `text` into the positioned glyph runs a glyph-based rasterizer will paint.
    fn shape_text(&self, text: &str, font_info: &FontInfo, rect_width: f64, available_width: f64) -> ShapedText {
        let Some(ref fs) = self.font_system else {
            return ShapedText::empty();
        };
        if text.is_empty() || font_info.size <= 0.0 {
            return ShapedText::empty();
        }
        let style = paint_text_style(font_info, rect_width, available_width);
        fs.lock().shape(text, &style)
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
            // A promoted layer (faded by group opacity, or pinned/sticky) becomes a compositing
            // group the scene backend fades + positions as a unit. The base scroll layer at full
            // opacity needs no wrapper.
            let promoted = layer.opacity < 1.0 || !matches!(layer.anchor, TileAnchor::Scroll);
            if promoted {
                out.push(PaintCommand::PushLayer {
                    opacity: layer.opacity,
                    anchor: layer.anchor,
                });
            }
            for &element_id in &layer.elements {
                out.extend(self.paint_element(element_id, state));
            }
            if promoted {
                out.push(PaintCommand::PopLayer);
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

    /// The element's CSS `mix-blend-mode`, applied to its painted boxes so backends blend
    /// them with the backdrop. Blending happens against whatever is already painted
    /// beneath the element (tile content for canvas backends, the scene for Vello) — the
    /// spec's stacking-context isolation rules are not modelled.
    fn mix_blend_mode(&self, node_id: NodeId) -> BlendMode {
        let doc = &self.layer_list.layout_tree.render_tree.doc;
        match doc.get_style(node_id, &StyleProperty::MixBlendMode) {
            Value::Keyword(kw) => BlendMode::from_css_keyword(&lookup(kw)),
            _ => BlendMode::Normal,
        }
    }

    /// The base fill for an element's background box plus any overlay `background-image`
    /// gradient layers to paint on top, back-to-front.
    ///
    /// A lone non-tiled `linear-gradient(...)` becomes the base brush directly (preserving the
    /// historical single-gradient path where the border/radius decorate the same rect).
    /// Multiple layers — or any tiled layer (a repeated `background-size` cell) — instead stack
    /// as separate rects over the solid `background-color`.
    fn background_fill(&self, node_id: NodeId) -> (Brush, Vec<Gradient>) {
        let doc = &self.layer_list.layout_tree.render_tree.doc;
        let layers = doc.background_layers(node_id);
        let color = self.get_brush(node_id, &StyleProperty::BackgroundColor, Brush::solid(Color::TRANSPARENT));
        match layers.as_slice() {
            [] => (color, Vec::new()),
            [Gradient::Linear(g)] if g.tiling.is_none() => (Brush::gradient(Gradient::Linear(g.clone())), Vec::new()),
            _ => (color, layers),
        }
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
                let r = Rectangle::new(border_box)
                    .with_background(brush)
                    .with_blend_mode(self.mix_blend_mode(dom_node_id));
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
                let shaped = self.shape_text(&ctx.text, &ctx.font_info, r.width, avail_w);
                let t = Text::new(r, &ctx.text, &ctx.font_info, brush, avail_w, shaped);
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
                let r = Rectangle::new(layout_element.box_model.border_box)
                    .with_background(brush)
                    .with_blend_mode(self.mix_blend_mode(dom_node_id));
                let r = self.decorate_with_border_and_radius(dom_node_id, r);
                commands.push(PaintCommand::rectangle(r));
            }
            ElementContext::None => {
                let (brush, overlay_layers) = self.background_fill(dom_node_id);
                let border_box = layout_element.box_model.border_box;
                let r = Rectangle::new(border_box)
                    .with_background(brush)
                    .with_blend_mode(self.mix_blend_mode(dom_node_id));
                let r = self.decorate_with_border_and_radius(dom_node_id, r);
                commands.push(PaintCommand::rectangle(r));

                // background-image paints on top of the background-color.
                if let Some(bg) = bg_media {
                    commands.extend(self.background_media_commands(bg, layout_element, dom_node_id));
                }

                // Stacked gradient layers (multi-layer / tiled backgrounds, e.g. a CSS
                // checkerboard). CSS paints the first-listed layer on top, so emit them
                // back-to-front over the base fill.
                let blend = self.mix_blend_mode(dom_node_id);
                for layer in overlay_layers.into_iter().rev() {
                    let r = Rectangle::new(border_box)
                        .with_background(Brush::gradient(layer))
                        .with_blend_mode(blend);
                    commands.push(PaintCommand::rectangle(r));
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
