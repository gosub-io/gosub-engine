pub mod commands;
pub mod text_field;

use crate::common::browser_state::{BrowserState, WireframeState};
use crate::common::document::node::NodeId;
use crate::common::document::pipeline_doc::{BgImageLayout, BgSize};
use crate::common::document::style::{lookup, BorderStyle as CssBorderStyle, Display, StyleProperty, Value};
use crate::common::font::{FontAlignment, FontInfo};
use crate::common::geo::Rect;
use crate::common::media::MediaStore;
use crate::layering::layer::LayerList;
use crate::layouter::{
    BackgroundMedia, ElementContext, ElementContextFormControl, ElementContextSelectPopup, FormControl,
    LayoutElementId, LayoutElementNode, MeterLevel, PopupRow,
};
use crate::painter::commands::border::{Border, BorderStyle};
use crate::painter::commands::brush::Brush;
use crate::painter::commands::color::Color;
use crate::painter::commands::gradient::{Gradient, Tiling};
use crate::painter::commands::rectangle::{BlendMode, Radius, Rectangle};
use crate::painter::commands::text::Text;
use crate::painter::commands::PaintCommand;
use crate::render::backend::TileAnchor;
use crate::tiler::TiledLayoutElement;
use gosub_interface::document::ControlEditState;
use gosub_interface::font::FontStyle;
use gosub_interface::font_system::{FontStretch, FontSystem, FontWeight, ShapedText, TextAlign, TextStyle};
use parking_lot::Mutex;
use std::sync::Arc;

/// A whole-viewport paint command list for the GPU-scene path, translated by a backend's `render`
/// into its native scene. Replaces the tile/rasterize/composite stages for GPU backends.
pub struct PaintScene {
    /// Paint order, bottom layer first.
    pub commands: Vec<PaintCommand>,
    pub media_store: Arc<MediaStore>,
    /// Full laid-out page height in CSS pixels (for scroll clamping on the host).
    pub page_height: f64,
}

/// The same [`TextStyle`] mapping the layouter measured with, so shaping reproduces its box.
///
/// Start-aligned text wraps at the layouter's container width to reproduce its line breaks (a
/// fragment can carry a whole multi-line paragraph). Center/End/Justify wrap at the fragment's
/// own box instead - glyphs shifted outside it would land in tiles that never repaint the command.
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

/// Turns the layout tree into paint commands for the renderer.
pub struct Painter {
    layer_list: Arc<LayerList>,
    /// The same instance the layouter measured with; text is shaped here once at command-build
    /// time. `None` (e.g. the null backend) yields empty glyph runs, drawable only by
    /// engine-native text rasterizers.
    font_system: Option<Arc<Mutex<dyn FontSystem>>>,
    /// Commands per element for this paint pass. An element straddling several tiles is asked for
    /// its commands once per tile; they don't depend on the tile (page coordinates), and shaping
    /// a textarea's text six times per keystroke is what made typing feel slow.
    memo: Mutex<std::collections::HashMap<LayoutElementId, Vec<PaintCommand>>>,
}

impl Painter {
    pub fn new(layer_list: Arc<LayerList>, font_system: Option<Arc<Mutex<dyn FontSystem>>>) -> Painter {
        Painter {
            layer_list,
            font_system,
            memo: Mutex::new(std::collections::HashMap::new()),
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

    pub fn paint(&self, element: &TiledLayoutElement, state: &BrowserState) -> Vec<PaintCommand> {
        if let Some(cmds) = self.memo.lock().get(&element.id) {
            return cmds.clone();
        }
        let cmds = self.paint_element(element.id, state);
        self.memo.lock().insert(element.id, cmds.clone());
        cmds
    }

    /// Flattens every element into one command list, in z-order (`layer_ids`) then paint order
    /// (`layer.elements`) - matching the tiler's z-ordering. For GPU-scene backends that render
    /// the whole viewport in one pass.
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

    pub fn paint_element(&self, element_id: LayoutElementId, state: &BrowserState) -> Vec<PaintCommand> {
        let mut commands = Vec::new();

        let Some(layout_element) = self.layer_list.layout_tree.get_node_by_id(element_id) else {
            return Vec::new();
        };
        let dom_node_id = layout_element.dom_node_id;

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

    /// Scales a brush's alpha by the element's `opacity`. Per-element approximation - no group
    /// compositing with descendants; exact for leaf boxes, approximate otherwise.
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

    /// The element's CSS `mix-blend-mode`. Blends against whatever is already painted beneath it
    /// (tile content for canvas backends, the scene for Vello) - stacking-context isolation is
    /// not modelled.
    fn mix_blend_mode(&self, node_id: NodeId) -> BlendMode {
        let doc = &self.layer_list.layout_tree.render_tree.doc;
        match doc.get_style(node_id, &StyleProperty::MixBlendMode) {
            Value::Keyword(kw) => BlendMode::from_css_keyword(&lookup(kw)),
            _ => BlendMode::Normal,
        }
    }

    /// Base fill plus overlay `background-image` gradient layers to paint on top, back-to-front.
    ///
    /// A lone non-tiled gradient becomes the base brush directly, so border/radius decorate the
    /// same rect. Multiple or tiled layers instead stack as separate rects over `background-color`.
    fn background_fill(&self, node_id: NodeId) -> (Brush, Vec<Gradient>) {
        let doc = &self.layer_list.layout_tree.render_tree.doc;
        let layers = doc.background_layers(node_id);
        let color = self.get_brush(
            node_id,
            &StyleProperty::BackgroundColor,
            Brush::solid(Color::TRANSPARENT),
        );
        match layers.as_slice() {
            [] => (color, Vec::new()),
            [Gradient::Linear(g)] if g.tiling.is_none() => (Brush::gradient(Gradient::Linear(g.clone())), Vec::new()),
            _ => (color, layers),
        }
    }

    /// Paints an image's `alt` text inside its box. `icon_offset_x` is the width taken by a
    /// broken-image icon at the top-left (0 if none), so the text starts past it.
    fn image_alt_command(
        &self,
        node_id: NodeId,
        alt: &str,
        border_box: Rect,
        icon_offset_x: f64,
    ) -> Option<PaintCommand> {
        const PAD: f64 = 3.0;
        let x = border_box.x + icon_offset_x + PAD;
        let y = border_box.y + PAD;
        let width = border_box.width - icon_offset_x - PAD * 2.0;
        let height = border_box.height - PAD * 2.0;
        if width <= 1.0 || height <= 1.0 {
            return None;
        }
        let rect = Rect::new(x, y, width, height);

        let font_info = self.alt_font_info(node_id);
        let brush = self.get_brush(node_id, &StyleProperty::Color, Brush::solid(Color::BLACK));
        let shaped = self.shape_text(alt, &font_info, rect.width, rect.width);
        Some(PaintCommand::text(Text::new(
            rect, alt, &font_info, brush, rect.width, shaped,
        )))
    }

    /// Minimal [`FontInfo`] for `alt` text: the element's computed family/size, start-aligned and
    /// undecorated, matching how browsers render the placeholder label.
    fn alt_font_info(&self, node_id: NodeId) -> FontInfo {
        let doc = &self.layer_list.layout_tree.render_tree.doc;
        let size = match doc.get_style(node_id, &StyleProperty::FontSize) {
            Value::Unit(px, _) => px as f64,
            _ => 16.0,
        };
        let family = match doc.get_style(node_id, &StyleProperty::FontFamily) {
            Value::Keyword(id) => lookup(id),
            _ => "sans-serif".to_string(),
        };
        FontInfo {
            family,
            size,
            weight: 400,
            width: 100,
            slant: 0,
            line_height: size * 1.4,
            letter_spacing: 0.0,
            alignment: FontAlignment::Start,
            underline: false,
            line_through: false,
        }
    }

    fn get_parent_brush(&self, node_id: NodeId, css_prop: &StyleProperty, default: Brush) -> Brush {
        let doc = &self.layer_list.layout_tree.render_tree.doc;
        match doc.parent(node_id) {
            Some(parent_id) => self.get_brush(parent_id, css_prop, default),
            None => default,
        }
    }

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
            BackgroundMedia::Image {
                media_id,
                natural,
                layout,
            } => {
                // Finalize the tile geometry now that the box is known (cover/contain need it).
                let tiling = compute_bg_tiling(natural, &layout, border_box.width as f32, border_box.height as f32);
                let brush = Brush::image_tiled(media_id, tiling);
                let r = Rectangle::new(border_box)
                    .with_background(brush)
                    .with_blend_mode(self.mix_blend_mode(dom_node_id));
                let r = self.decorate_with_border_and_radius(dom_node_id, r);
                vec![PaintCommand::rectangle(r)]
            }
            BackgroundMedia::Svg(media_id) => vec![PaintCommand::svg(media_id, Rectangle::new(border_box))],
        }
    }

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
                let border_box = layout_element.box_model.border_box;
                let blend = self.mix_blend_mode(dom_node_id);

                // CSS paints background-color behind the (possibly transparent) replaced content,
                // e.g. a transparent PNG on `<img style="background:#3a7">` shows green through.
                let (bg_brush, _) = self.background_fill(dom_node_id);
                if !matches!(&bg_brush, Brush::Solid(c) if c.a() == 0.0) {
                    let bg_r = Rectangle::new(border_box)
                        .with_background(bg_brush)
                        .with_blend_mode(blend);
                    commands.push(PaintCommand::rectangle(
                        self.decorate_with_border_and_radius(dom_node_id, bg_r),
                    ));
                }

                let brush = Brush::image(image_ctx.media_id);
                // A broken-image placeholder is drawn at its natural icon size in the top-left of
                // the reserved box (like Firefox) rather than stretched to fill it.
                let draw_box = if image_ctx.placeholder {
                    let iw = (image_ctx.dimension.width).min(border_box.width);
                    let ih = (image_ctx.dimension.height).min(border_box.height);
                    Rect::new(border_box.x, border_box.y, iw, ih)
                } else {
                    border_box
                };
                let r = Rectangle::new(draw_box).with_background(brush).with_blend_mode(blend);
                // The border/radius belongs to the element box, not the shrunk icon rect.
                let border_target = if image_ctx.placeholder { border_box } else { draw_box };
                let border_r = self.decorate_with_border_and_radius(dom_node_id, Rectangle::new(border_target));
                if image_ctx.placeholder {
                    commands.push(PaintCommand::rectangle(r));
                    // Emit the element border separately so it frames the full reserved box.
                    if self.has_border(dom_node_id) {
                        commands.push(PaintCommand::rectangle(border_r));
                    }
                } else {
                    let r = self.decorate_with_border_and_radius(dom_node_id, r);
                    commands.push(PaintCommand::rectangle(r));
                }

                // Browsers show `alt` inside the box when the image renders nothing visible. For a
                // placeholder it sits right of the broken-image icon, else at the box's top-left.
                if let Some(alt) = &image_ctx.alt {
                    let icon_w = if image_ctx.placeholder { draw_box.width } else { 0.0 };
                    if let Some(cmd) = self.image_alt_command(dom_node_id, alt, border_box, icon_w) {
                        commands.push(cmd);
                    }
                }
            }
            ElementContext::FormControl(fc) => {
                commands.extend(self.form_control_commands(fc, layout_element, dom_node_id));
            }
            ElementContext::SelectPopup(popup) => {
                commands.extend(self.select_popup_commands(popup, layout_element));
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

        // Text runs don't carry an outline; the owning element does. The popup shares the select's
        // node but isn't the select.
        if !matches!(
            layout_element.context,
            ElementContext::Text(_) | ElementContext::SelectPopup(_)
        ) {
            if let Some(cmd) = self.outline_command(layout_element, dom_node_id) {
                commands.push(cmd);
            }
        }

        commands
    }

    /// CSS `outline`: a border-only rectangle around the border box, inflated by offset + width,
    /// following the element's corner radii. Takes no layout space.
    fn outline_command(&self, layout_element: &LayoutElementNode, dom_node_id: NodeId) -> Option<PaintCommand> {
        let doc = &self.layer_list.layout_tree.render_tree.doc;
        let width = doc.get_style_f32(dom_node_id, &StyleProperty::OutlineWidth) as f64;
        if width <= 0.0 {
            return None;
        }
        let style = match doc.get_style(dom_node_id, &StyleProperty::OutlineStyle) {
            Value::BorderStyle(s) if !matches!(s, CssBorderStyle::None | CssBorderStyle::Hidden) => {
                css_border_style_to_paint(&s)
            }
            _ => return None,
        };
        let offset = doc.get_style_f32(dom_node_id, &StyleProperty::OutlineOffset) as f64;
        // A negative offset pulls the ring inside the border box.
        let grow = offset + width;

        let bb = layout_element.box_model.border_box;
        let ring = Rect::new(bb.x - grow, bb.y - grow, bb.width + grow * 2.0, bb.height + grow * 2.0);
        if ring.width <= 0.0 || ring.height <= 0.0 {
            return None;
        }

        let brush = self.get_brush(dom_node_id, &StyleProperty::OutlineColor, Brush::solid(Color::BLACK));
        let border = Border::new(
            width as f32,
            style,
            [brush.clone(), brush.clone(), brush.clone(), brush],
        );
        let mut r = Rectangle::new(ring).with_border(border);

        // Radii grow with the box.
        let radius = |prop: &StyleProperty| {
            let v = doc.get_style_f32(dom_node_id, prop) as f64;
            if v > 0.0 {
                v + grow
            } else {
                0.0
            }
        };
        let (tl, tr, br, bl) = (
            radius(&StyleProperty::BorderTopLeftRadius),
            radius(&StyleProperty::BorderTopRightRadius),
            radius(&StyleProperty::BorderBottomRightRadius),
            radius(&StyleProperty::BorderBottomLeftRadius),
        );
        if tl > 0.0 || tr > 0.0 || br > 0.0 || bl > 0.0 {
            r = r.with_radius_tlrb(Radius::new(tl), Radius::new(tr), Radius::new(br), Radius::new(bl));
        }
        Some(PaintCommand::rectangle(r))
    }

    /// An open `<select>` dropdown per the design guide: shadowed white box, 30px rows, light
    /// hover / solid keyboard-active highlight, checkmark on the committed value, muted group
    /// labels and disabled options, scrollbar when needed.
    fn select_popup_commands(
        &self,
        popup: &ElementContextSelectPopup,
        layout_element: &LayoutElementNode,
    ) -> Vec<PaintCommand> {
        let doc = &self.layer_list.layout_tree.render_tree.doc;
        let bb = layout_element.box_model.border_box;
        let mb = layout_element.box_model.margin_box;
        let inner = layout_element.box_model.content_box;
        let mut commands = Vec::new();

        let theme = crate::common::theme::select_theme();
        if let Some(shadow) = popup.shadow {
            commands.push(PaintCommand::svg(shadow, Rectangle::new(mb)));
        }
        let border = Brush::solid(theme.popup_border.clone());
        commands.push(PaintCommand::rectangle(
            Rectangle::new(bb)
                .with_background(Brush::solid(theme.popup_bg.clone()))
                .with_border(Border::new(
                    1.0,
                    BorderStyle::Solid,
                    [border.clone(), border.clone(), border.clone(), border],
                ))
                .with_radius(Radius::new(6.0)),
        ));

        let open = doc.open_select();
        let (hovered, active, first) = open.map_or((None, None, 0), |o| (o.hover, o.active, o.first_row));
        let chosen = doc.selected_option(popup.select);
        let bar_w = popup.scrollbar_width();
        let mut ink_font = popup.font_info.clone();
        ink_font.alignment = FontAlignment::Start;

        for (i, row) in popup.rows.iter().enumerate().skip(first).take(popup.visible_rows) {
            let rect = Rect::new(
                inner.x,
                inner.y + (i - first) as f64 * popup.row_height,
                inner.width - bar_w,
                popup.row_height,
            );
            let (label, is_group, disabled, is_chosen) = match row {
                PopupRow::Group { label } => (label, true, false, false),
                PopupRow::Option {
                    label,
                    disabled,
                    node_id,
                } => (label, false, *disabled, Some(*node_id) == chosen),
            };
            let selectable = row.selectable();
            let is_active = selectable && active == Some(i);
            let is_hover = selectable && !is_active && hovered == Some(i);
            if is_active || is_hover {
                let bg = if is_active {
                    theme.active_bg.clone()
                } else {
                    theme.hover_bg.clone()
                };
                commands.push(PaintCommand::rectangle(
                    Rectangle::new(rect)
                        .with_background(Brush::solid(bg))
                        .with_radius(Radius::new(4.0)),
                ));
            }
            let color = if is_active {
                theme.active_text.clone()
            } else if is_group {
                theme.group_text.clone()
            } else if disabled {
                theme.disabled_text.clone()
            } else {
                theme.text.clone()
            };
            let text = if is_group {
                format!("\u{2014} {label} \u{2014}")
            } else {
                label.clone()
            };
            let text_rect = Rect::new(
                rect.x + 10.0,
                rect.y + 6.0,
                (rect.width - 20.0 - 24.0).max(1.0),
                rect.height - 12.0,
            );
            let shaped = self.shape_text(&text, &ink_font, text_rect.width, 1_000_000_000.0);
            commands.push(PaintCommand::text(Text::new(
                text_rect,
                &text,
                &ink_font,
                Brush::solid(color),
                1_000_000_000.0,
                shaped,
            )));
            if is_chosen && !is_active {
                if let Some(check) = popup.check {
                    let s = 16.0;
                    commands.push(PaintCommand::svg(
                        check,
                        Rectangle::new(Rect::new(
                            rect.x + rect.width - 10.0 - s,
                            rect.y + (rect.height - s) / 2.0,
                            s,
                            s,
                        )),
                    ));
                }
            }
        }

        if let Some((track, thumb)) = popup.scrollbar(inner, first) {
            commands.push(PaintCommand::rectangle(
                Rectangle::new(track).with_background(Brush::solid(theme.scrollbar_track.clone())),
            ));
            commands.push(PaintCommand::rectangle(
                Rectangle::new(thumb)
                    .with_background(Brush::solid(theme.scrollbar_thumb.clone()))
                    .with_radius(Radius::new(4.0)),
            ));
        }
        commands
    }

    /// A native form control: the CSS chrome (background + border) with the widget's state on top,
    /// composed from rectangles and shaped text.
    fn form_control_commands(
        &self,
        fc: &ElementContextFormControl,
        layout_element: &LayoutElementNode,
        dom_node_id: NodeId,
    ) -> Vec<PaintCommand> {
        let border_box = layout_element.box_model.border_box;
        let content_box = layout_element.box_model.content_box;
        let blend = self.mix_blend_mode(dom_node_id);
        let mut commands = Vec::new();

        let (bg_brush, _) = self.background_fill(dom_node_id);
        let chrome = Rectangle::new(border_box)
            .with_background(bg_brush)
            .with_blend_mode(blend);
        let chrome = self.decorate_with_border_and_radius(dom_node_id, chrome);
        commands.push(PaintCommand::rectangle(chrome));

        // Solid fill respecting the element's opacity.
        let fill = |c: Color| self.apply_opacity(dom_node_id, Brush::solid(c));
        // Gosub blue, matching the checkbox/radio artwork.
        let accent = if fc.disabled {
            Color::from_rgb8(0xb0, 0xb0, 0xb0)
        } else {
            Color::from_rgb8(0x23, 0x82, 0xeb)
        };
        let track_gray = Color::from_rgb8(0xe6, 0xe6, 0xe6);
        // One line of text, vertically centered in the content box.
        let line_rect = |inset_x: f64| {
            let h = fc.font_info.line_height.max(fc.font_info.size);
            Rect::new(
                content_box.x + inset_x,
                content_box.y + ((content_box.height - h) / 2.0).max(0.0),
                (content_box.width - inset_x * 2.0).max(1.0),
                h,
            )
        };
        let css_text_brush = || {
            self.apply_opacity(
                dom_node_id,
                self.get_brush(dom_node_id, &StyleProperty::Color, Brush::solid(Color::BLACK)),
            )
        };

        match &fc.control {
            FormControl::TextField {
                value: initial_value,
                placeholder,
                masked,
                multiline,
                grip,
                ..
            } => {
                if let Some(grip) = grip {
                    let pb = layout_element.box_model.padding_box;
                    let g = 12.0_f64.min(pb.width).min(pb.height);
                    commands.push(PaintCommand::svg(
                        *grip,
                        Rectangle::new(Rect::new(pb.x + pb.width - g - 1.0, pb.y + pb.height - g - 1.0, g, g)),
                    ));
                }
                // The typed value/caret/selection are read here, not at layout, so typing is
                // paint-only.
                let doc = &self.layer_list.layout_tree.render_tree.doc;
                let focused = doc.is_focused(dom_node_id);
                let state = doc
                    .control_edit_state(dom_node_id)
                    .unwrap_or_else(|| ControlEditState::new(initial_value.clone(), initial_value.chars().count()));
                let value = state.value.clone();
                let caret = focused.then_some(state.caret);
                let selection = if focused { state.selection() } else { None };
                let is_placeholder = value.is_empty();
                let mut text = if is_placeholder {
                    placeholder.clone()
                } else if *masked {
                    "\u{2022}".repeat(value.chars().count())
                } else {
                    value.clone()
                };
                let brush = if is_placeholder {
                    fill(Color::from_rgb8(0x75, 0x75, 0x75))
                } else {
                    css_text_brush()
                };
                let theme = crate::common::theme::select_theme();
                let selection_brush = Brush::solid(theme.selection_bg.clone());
                let inset_x = text_field::inset_x(content_box.width);
                let mut rect = if *multiline {
                    Rect::new(
                        content_box.x + inset_x,
                        content_box.y,
                        (content_box.width - inset_x * 2.0).max(1.0),
                        content_box.height.max(fc.font_info.line_height),
                    )
                } else {
                    line_rect(inset_x)
                };
                let avail = if *multiline { rect.width } else { 1_000_000_000.0 };
                let line_h = fc.font_info.line_height.max(fc.font_info.size);
                // No caret index into a placeholder; it sits at the start.
                let mut caret = caret.map(|c| if is_placeholder { 0 } else { c.min(text.chars().count()) });

                // Paint commands can't clip, so cut the text to what fits (see `text_field`).
                let mut caret_pos: Option<(f64, f64)> = None;
                let mut rows_to_draw: Vec<(String, f64)> = Vec::new();
                let mut highlights: Vec<Rect> = Vec::new();
                if let Some(fs) = &self.font_system {
                    let mut fs = fs.lock();
                    if *multiline {
                        // Visual rows (greedy wrap) drawn one by one; the window is the stored
                        // scroll row, nudged so the caret stays in view.
                        let area = text_field::area_layout(&mut *fs, &text, &fc.font_info, content_box);
                        rect = area.text;
                        let caret_row = caret.map(|c| text_field::row_of_caret(&area.rows, c));
                        let first = match caret_row {
                            Some(r) => area.first_showing(state.scroll, r),
                            None => area.clamp_first(state.scroll),
                        };
                        for (i, row) in area.rows.iter().enumerate().skip(first).take(area.rows_fit) {
                            let dy = (i - first) as f64 * line_h;
                            let rt = text_field::row_text(&text, row);
                            if let Some((x0, x1)) = selection
                                .and_then(|sel| text_field::selection_in_row(&mut *fs, &text, row, sel, &fc.font_info))
                            {
                                let x1 = x1.min(rect.width);
                                highlights.push(Rect::new(rect.x + x0, rect.y + dy, (x1 - x0).max(0.0), line_h));
                            }
                            if !rt.is_empty() {
                                rows_to_draw.push((rt, dy));
                            }
                        }
                        if let (Some(c), Some(caret_row)) = (caret, caret_row) {
                            let row = &area.rows[caret_row];
                            let rt = text_field::row_text(&text, row);
                            let x = text_field::x_in_row(&mut *fs, &rt, c - row.start, &fc.font_info);
                            caret_pos = Some((x.min(rect.width), (caret_row - first) as f64 * line_h));
                        }
                        if let (Some(track), Some(thumb)) = (area.track, area.thumb(first)) {
                            commands.push(PaintCommand::rectangle(
                                Rectangle::new(track).with_background(Brush::solid(theme.scrollbar_track.clone())),
                            ));
                            commands.push(PaintCommand::rectangle(
                                Rectangle::new(thumb)
                                    .with_background(Brush::solid(theme.scrollbar_thumb.clone()))
                                    .with_radius(Radius::new(3.0)),
                            ));
                        }
                        text.clear();
                    } else {
                        let (start, end) =
                            text_field::single_line_window(&mut *fs, &text, caret, &fc.font_info, rect.width);
                        text = text.chars().skip(start).take(end - start).collect();
                        caret = caret.map(|c| c.saturating_sub(start).min(text.chars().count()));
                        if let Some((s, e)) = selection {
                            let row = text_field::Row {
                                start,
                                end,
                                hard_end: true,
                            };
                            let shown = text.chars().count();
                            let sel = (s.clamp(start, start + shown), e.clamp(start, start + shown));
                            // The window's text is `text` itself (already cut), so index it from 0.
                            let local = text_field::Row {
                                start: 0,
                                end: shown,
                                hard_end: row.hard_end,
                            };
                            if let Some((x0, x1)) = text_field::selection_in_row(
                                &mut *fs,
                                &text,
                                &local,
                                (sel.0 - start, sel.1 - start),
                                &fc.font_info,
                            ) {
                                let x1 = x1.min(rect.width);
                                highlights.push(Rect::new(rect.x + x0, rect.y, (x1 - x0).max(0.0), line_h));
                            }
                        }
                        if let Some(c) = caret {
                            caret_pos = Some(text_field::caret_offset(&mut *fs, &text, c, &fc.font_info, avail));
                        }
                    }
                }
                for hl in highlights {
                    if hl.width > 0.0 {
                        commands.push(PaintCommand::rectangle(
                            Rectangle::new(hl).with_background(selection_brush.clone()),
                        ));
                    }
                }
                for (row_text, dy) in rows_to_draw {
                    let row_rect = Rect::new(rect.x, rect.y + dy, rect.width, line_h);
                    let shaped = self.shape_text(&row_text, &fc.font_info, row_rect.width, 1_000_000_000.0);
                    commands.push(PaintCommand::text(Text::new(
                        row_rect,
                        &row_text,
                        &fc.font_info,
                        brush.clone(),
                        1_000_000_000.0,
                        shaped,
                    )));
                }

                if !text.is_empty() {
                    let shaped = self.shape_text(&text, &fc.font_info, rect.width, avail);
                    commands.push(PaintCommand::text(Text::new(
                        rect,
                        &text,
                        &fc.font_info,
                        brush,
                        avail,
                        shaped,
                    )));
                }
                if let Some((cx, cy)) = caret_pos {
                    let caret_rect = Rect::new((rect.x + cx).min(rect.x + rect.width - 1.0), rect.y + cy, 1.0, line_h);
                    commands.push(PaintCommand::rectangle(
                        Rectangle::new(caret_rect)
                            .with_background(css_text_brush())
                            .with_blend_mode(blend),
                    ));
                }
            }
            FormControl::Button { label } => {
                if label.is_empty() {
                    return commands;
                }
                let mut font_info = fc.font_info.clone();
                font_info.alignment = FontAlignment::Center;
                let rect = line_rect(0.0);
                let shaped = self.shape_text(label, &font_info, rect.width, rect.width);
                commands.push(PaintCommand::text(Text::new(
                    rect,
                    label,
                    &font_info,
                    css_text_brush(),
                    rect.width,
                    shaped,
                )));
            }
            FormControl::Select { label, chevron } => {
                // Label in the content area; chevron centred in the 28px arrow area at the right
                // (the padding the UA sheet reserves for it).
                let pb = layout_element.box_model.padding_box;
                let arrow_w = 28.0_f64.min(pb.width / 2.0);
                if !label.is_empty() {
                    let rect = line_rect(0.0);
                    let shaped = self.shape_text(label, &fc.font_info, rect.width, 1_000_000_000.0);
                    commands.push(PaintCommand::text(Text::new(
                        rect,
                        label,
                        &fc.font_info,
                        css_text_brush(),
                        1_000_000_000.0,
                        shaped,
                    )));
                }
                if let Some(chevron) = chevron {
                    let s = 16.0_f64.min(pb.height);
                    commands.push(PaintCommand::svg(
                        *chevron,
                        Rectangle::new(Rect::new(
                            pb.x + pb.width - arrow_w + (arrow_w - s) / 2.0,
                            pb.y + (pb.height - s) / 2.0,
                            s,
                            s,
                        )),
                    ));
                }
            }
            FormControl::Checkbox(icons) | FormControl::Radio(icons) => {
                let doc = &self.layer_list.layout_tree.render_tree.doc;
                let icon = icons.pick(doc.is_checked(dom_node_id), fc.disabled);
                let side = content_box.width.min(content_box.height).max(1.0);
                let box_rect = Rect::new(
                    content_box.x + (content_box.width - side) / 2.0,
                    content_box.y + (content_box.height - side) / 2.0,
                    side,
                    side,
                );
                commands.push(PaintCommand::svg(icon, Rectangle::new(box_rect)));
            }
            FormControl::Range { min, max, fraction } => {
                let live = self
                    .layer_list
                    .layout_tree
                    .render_tree
                    .doc
                    .control_edit_state(dom_node_id)
                    .and_then(|s| s.value.trim().parse::<f64>().ok())
                    .filter(|_| max > min)
                    .map(|v| ((v - min) / (max - min)).clamp(0.0, 1.0));
                let fraction = &live.unwrap_or(*fraction);
                let cy = content_box.y + content_box.height / 2.0;
                let track_h = 4.0_f64.min(content_box.height);
                let track = Rect::new(content_box.x, cy - track_h / 2.0, content_box.width.max(1.0), track_h);
                commands.push(PaintCommand::rectangle(
                    Rectangle::new(track)
                        .with_background(fill(track_gray))
                        .with_radius(Radius::new(track_h / 2.0))
                        .with_blend_mode(blend),
                ));
                let active_w = track.width * fraction;
                if active_w > 0.5 {
                    commands.push(PaintCommand::rectangle(
                        Rectangle::new(Rect::new(track.x, track.y, active_w, track_h))
                            .with_background(fill(accent.clone()))
                            .with_radius(Radius::new(track_h / 2.0))
                            .with_blend_mode(blend),
                    ));
                }
                let d = 12.0_f64.min(content_box.height);
                let thumb_x = content_box.x + (content_box.width - d).max(0.0) * fraction;
                commands.push(PaintCommand::rectangle(
                    Rectangle::new(Rect::new(thumb_x, cy - d / 2.0, d, d))
                        .with_background(fill(accent.clone()))
                        .with_radius(Radius::new(d / 2.0))
                        .with_blend_mode(blend),
                ));
            }
            FormControl::Progress { fraction } => {
                let radius = Radius::new(content_box.height / 2.0);
                commands.push(PaintCommand::rectangle(
                    Rectangle::new(content_box)
                        .with_background(fill(track_gray))
                        .with_radius(radius)
                        .with_blend_mode(blend),
                ));
                let bar = match fraction {
                    Some(f) => Rect::new(content_box.x, content_box.y, content_box.width * f, content_box.height),
                    // Static stand-in for the animated indeterminate bar.
                    None => Rect::new(
                        content_box.x + content_box.width * 0.3,
                        content_box.y,
                        content_box.width * 0.4,
                        content_box.height,
                    ),
                };
                if bar.width > 0.5 {
                    commands.push(PaintCommand::rectangle(
                        Rectangle::new(bar)
                            .with_background(fill(accent.clone()))
                            .with_radius(radius)
                            .with_blend_mode(blend),
                    ));
                }
            }
            FormControl::Meter { fraction, level } => {
                let radius = Radius::new(content_box.height / 2.0);
                commands.push(PaintCommand::rectangle(
                    Rectangle::new(content_box)
                        .with_background(fill(track_gray))
                        .with_radius(radius)
                        .with_blend_mode(blend),
                ));
                let color = match level {
                    MeterLevel::Optimum => Color::from_rgb8(0x10, 0x7c, 0x10),
                    MeterLevel::Suboptimum => Color::from_rgb8(0xff, 0xb9, 0x00),
                    MeterLevel::Critical => Color::from_rgb8(0xd8, 0x3b, 0x01),
                };
                let bar_w = content_box.width * fraction;
                if bar_w > 0.5 {
                    commands.push(PaintCommand::rectangle(
                        Rectangle::new(Rect::new(content_box.x, content_box.y, bar_w, content_box.height))
                            .with_background(fill(color))
                            .with_radius(radius)
                            .with_blend_mode(blend),
                    ));
                }
            }
            FormControl::ColorSwatch { value } => {
                let color = Color::try_from_css(value).unwrap_or(Color::BLACK);
                commands.push(PaintCommand::rectangle(
                    Rectangle::new(content_box)
                        .with_background(fill(color))
                        .with_blend_mode(blend),
                ));
            }
        }

        commands
    }

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

/// Resolves `background-size`/`-position` into a [`Tiling`], now that the border box is known.
/// `cover`/`contain` yield a single aspect-preserved tile (no repeat), so the backend paints it
/// once and lets the box clip (cover) or the background-color show (contain).
fn compute_bg_tiling(natural: (f32, f32), layout: &BgImageLayout, box_w: f32, box_h: f32) -> Option<Tiling> {
    let (nw, nh) = natural;
    if nw <= 0.0 || nh <= 0.0 || box_w <= 0.0 || box_h <= 0.0 {
        return None;
    }

    let (tw, th) = match layout.size {
        BgSize::Auto => (nw, nh),
        BgSize::Length(w, h) => (w, h),
        // Preserve aspect: contain fits inside the box, cover fills it.
        BgSize::Contain => {
            let s = (box_w / nw).min(box_h / nh);
            (nw * s, nh * s)
        }
        BgSize::Cover => {
            let s = (box_w / nw).max(box_h / nh);
            (nw * s, nh * s)
        }
    };
    if tw <= 0.0 || th <= 0.0 {
        return None;
    }

    let px = if layout.center.0 {
        (box_w - tw) / 2.0
    } else {
        layout.position.0
    };
    let py = if layout.center.1 {
        (box_h - th) / 2.0
    } else {
        layout.position.1
    };

    Some(Tiling {
        tile_size: (tw, th),
        position: (px, py),
        repeat: layout.repeat,
    })
}
