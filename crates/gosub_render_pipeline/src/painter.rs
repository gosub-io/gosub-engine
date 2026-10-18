pub mod commands;
pub mod shape_cache;
pub mod text_field;

use crate::common::browser_state::{BrowserState, WireframeState};
use crate::common::document::node::NodeId;
use gosub_interface::style::{
    Anchor, BackgroundImage, BackgroundRepeat, BackgroundSize, BackgroundTile, BorderCollapse,
    BorderStyle as CssBorderStyle, Color as CssColor, ComputedStyle, Display, LinearGradient as CssLinearGradient,
    ObjectFit,
};

/// Which colour of a [`ComputedStyle`] a brush is made from. A function pointer rather than a
/// property id: the field is the property, now that the style is a struct.
type ColorOf = fn(&ComputedStyle) -> CssColor;

/// Which border style of a [`ComputedStyle`] an edge is drawn in.
type BorderStyleOf = fn(&ComputedStyle) -> CssBorderStyle;
use crate::common::font::{FontAlignment, FontInfo};
use crate::common::geo::Rect;
use crate::common::media::MediaStore;
use crate::layering::layer::LayerList;
use crate::layouter::{
    fraction_between, BackgroundMedia, CollapsedCellBorders, ElementContext, ElementContextFormControl,
    ElementContextSelectPopup, FormControl, LayoutElementId, LayoutElementNode, MeterLevel, PopupRow,
};
use crate::painter::commands::border::{Border, BorderStyle};
use crate::painter::commands::brush::Brush;
use crate::painter::commands::color::Color;
use crate::painter::commands::gradient::{ColorStop, Gradient, LinearGradient, Tiling};
use crate::painter::commands::rectangle::{BlendMode, Radius, Rectangle};
use crate::painter::commands::shadow::{RoundedRect, Shadow};
use crate::painter::commands::text::Text;
use crate::painter::commands::PaintCommand;
use crate::painter::shape_cache::ShapeCache;
use crate::render::backend::TileAnchor;
use crate::tiler::TiledLayoutElement;
use gosub_interface::document::ControlEditState;
use gosub_interface::font::FontStyle;
use gosub_interface::font_system::{FontStretch, FontSystem, FontWeight, ShapedText, TextAlign, TextStyle};
use gosub_interface::used;
use gosub_shared::microsyntax::parse_floating_point_number;
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
        line_height: font_info.line_height.map(|v| v as f32),
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
    /// Shaped runs shared with the passes before and after this one. The per-pass `memo` above
    /// stops at the pass boundary, and a scrolling page keeps handing the next pass the runs at
    /// the window edge that the last one already shaped. `None` when the caller has no grid to
    /// hang a cache on (a one-shot whole-page paint), and then every run is shaped here.
    shape_cache: Option<Arc<ShapeCache>>,
}

impl Painter {
    pub fn new(layer_list: Arc<LayerList>, font_system: Option<Arc<Mutex<dyn FontSystem>>>) -> Painter {
        Painter {
            layer_list,
            font_system,
            memo: Mutex::new(std::collections::HashMap::new()),
            shape_cache: None,
        }
    }

    /// Share `cache` with the other passes over the same tile grid, so a run at the window edge
    /// is shaped by the first pass that reaches it and read by the rest.
    #[must_use]
    pub fn with_shape_cache(mut self, cache: Arc<ShapeCache>) -> Painter {
        self.shape_cache = Some(cache);
        self
    }

    /// Shape `text` into the positioned glyph runs a glyph-based rasterizer will paint.
    ///
    /// Shared rather than copied: the same run reaches every tile the element covers, and with a
    /// grid cache every pass over those tiles as well.
    fn shape_text(&self, text: &str, font_info: &FontInfo, rect_width: f64, available_width: f64) -> Arc<ShapedText> {
        let Some(ref fs) = self.font_system else {
            return Arc::new(ShapedText::empty());
        };
        if text.is_empty() || font_info.size <= 0.0 {
            return Arc::new(ShapedText::empty());
        }
        let style = paint_text_style(font_info, rect_width, available_width);
        match self.shape_cache {
            Some(ref cache) => cache.get_or_shape(text, &style, || fs.lock().shape(text, &style)),
            None => Arc::new(fs.lock().shape(text, &style)),
        }
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

        // A `visibility: hidden` box keeps its place in layout and paints nothing of its own.
        // Its children are elements of their own: they inherit the hidden and stay unpainted,
        // unless one says `visible` again outside any `force-hidden` box.
        let doc = &self.layer_list.layout_tree.render_tree.doc;
        let visible = doc.computed_style(dom_node_id).is_visible();

        match state.wireframed {
            WireframeState::Only => {
                commands.extend(self.generate_wireframe_commands(layout_element));
            }
            WireframeState::Both => {
                if visible {
                    commands.extend(self.generate_element_commands(layout_element, dom_node_id));
                }
                commands.extend(self.generate_wireframe_commands(layout_element));
            }
            WireframeState::None => {
                if visible {
                    commands.extend(self.generate_element_commands(layout_element, dom_node_id));
                }
            }
        }

        if state.debug_table_cells {
            commands.extend(self.generate_table_debug_commands(layout_element, dom_node_id));
        }

        commands
    }

    fn get_brush(&self, node_id: NodeId, color_of: ColorOf, _default: Brush) -> Brush {
        let doc = &self.layer_list.layout_tree.render_tree.doc;
        let color = color_of(&doc.computed_style(node_id));
        let brush = Brush::solid(Color::from_rgba8(color.r, color.g, color.b, color.a));
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
        let opacity = doc.computed_style(node_id).box_group.opacity;
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
        BlendMode::from_css_keyword(&doc.computed_style(node_id).box_group.mix_blend_mode)
    }

    /// Base fill plus overlay `background-image` gradient layers to paint on top, back-to-front.
    ///
    /// A lone non-tiled gradient becomes the base brush directly, so border/radius decorate the
    /// same rect. Multiple or tiled layers instead stack as separate rects over `background-color`.
    fn background_fill(&self, node_id: NodeId, box_size: (f32, f32)) -> (Brush, Vec<Gradient>) {
        let doc = &self.layer_list.layout_tree.render_tree.doc;
        let style = doc.computed_style(node_id);
        let layers: Vec<Gradient> = style
            .background
            .layers
            .iter()
            .filter_map(|layer| match &layer.image {
                BackgroundImage::Linear(gradient) => gradient_layer(gradient, &layer.tile, box_size),
                BackgroundImage::None | BackgroundImage::Url(_) | BackgroundImage::Other => None,
            })
            .collect();
        let color = self.get_brush(
            node_id,
            |style| style.background.color,
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
        let brush = self.get_brush(node_id, |style| style.inherited.color, Brush::solid(Color::BLACK));
        let shaped = self.shape_text(alt, &font_info, rect.width, rect.width);
        Some(PaintCommand::text(Text::new(
            rect, alt, &font_info, brush, rect.width, shaped,
        )))
    }

    /// Minimal [`FontInfo`] for `alt` text: the element's computed family/size, start-aligned and
    /// undecorated, matching how browsers render the placeholder label.
    fn alt_font_info(&self, node_id: NodeId) -> FontInfo {
        let doc = &self.layer_list.layout_tree.render_tree.doc;
        let style = doc.computed_style(node_id);
        let size = f64::from(style.inherited.font_size);
        let family = style.inherited.font_family.to_string();
        FontInfo {
            family,
            size,
            weight: 400,
            width: 100,
            slant: 0,
            line_height: None,
            letter_spacing: 0.0,
            alignment: FontAlignment::Start,
            underline: false,
            line_through: false,
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
        let color = match doc.computed_style(dom_node_id).declared_display() {
            Some(Display::Table | Display::InlineTable) => Color::from_rgb8(255, 0, 0),
            Some(Display::TableCell) => Color::from_rgb8(0, 180, 0),
            Some(Display::TableRow) => Color::from_rgb8(0, 0, 255),
            Some(Display::TableRowGroup | Display::TableHeaderGroup | Display::TableFooterGroup) => {
                Color::from_rgb8(160, 0, 200)
            }
            Some(Display::TableCaption) => Color::from_rgb8(255, 140, 0),
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
                // Collapsed cells get their border from the TableBorderOverlay.
                let r = if layout_element.collapsed_borders.is_some() {
                    r
                } else {
                    self.decorate_with_border_and_radius(dom_node_id, None, r)
                };
                vec![PaintCommand::rectangle(r)]
            }
            BackgroundMedia::Svg(media_id) => vec![PaintCommand::svg(media_id, Rectangle::new(border_box))],
        }
    }

    fn generate_element_commands(&self, layout_element: &LayoutElementNode, dom_node_id: NodeId) -> Vec<PaintCommand> {
        let mut commands = Vec::new();

        // A run of text has no box of its own to cast a shadow, the popup is not the select, and
        // the table's border overlay paints its cells' borders, not the table.
        let (outer_shadows, mut inset_shadows) = match layout_element.context {
            ElementContext::Text(_) | ElementContext::SelectPopup(_) | ElementContext::TableBorderOverlay(_) => {
                (None, None)
            }
            _ => self.shadow_commands(layout_element, dom_node_id),
        };
        commands.extend(outer_shadows);

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
                // A text node's own style inherits `color` from its style parent, which is not
                // its box parent when that is a `display: contents` element or a slot.
                let brush = self.get_brush(dom_node_id, |style| style.inherited.color, Brush::solid(Color::BLACK));
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
                commands.extend(inset_shadows.take());
                commands.push(PaintCommand::svg(svg_ctx.media_id, Rectangle::new(border_box)));
                // The SVG painter doesn't draw the element's CSS border/radius, so emit it as a
                // separate border-only rectangle painted on top of the icon (e.g. the HN logo's
                // `border:1px white solid`).
                if self.has_border(dom_node_id) {
                    let r = self.decorate_with_border_and_radius(dom_node_id, None, Rectangle::new(border_box));
                    commands.push(PaintCommand::rectangle(r));
                }
            }
            ElementContext::Image(image_ctx) => {
                let border_box = layout_element.box_model.border_box;
                let blend = self.mix_blend_mode(dom_node_id);

                // CSS paints background-color behind the (possibly transparent) replaced content,
                // e.g. a transparent PNG on `<img style="background:#3a7">` shows green through.
                let (bg_brush, _) =
                    self.background_fill(dom_node_id, (border_box.width as f32, border_box.height as f32));
                if !matches!(&bg_brush, Brush::Solid(c) if c.a() == 0.0) {
                    let bg_r = Rectangle::new(border_box)
                        .with_background(bg_brush)
                        .with_blend_mode(blend);
                    commands.push(PaintCommand::rectangle(self.decorate_with_border_and_radius(
                        dom_node_id,
                        None,
                        bg_r,
                    )));
                }

                // An inset shadow sits over the background and under the image.
                commands.extend(inset_shadows.take());

                // Any `object-fit` but `fill` sizes the image into the content box by its own
                // aspect ratio and anchors it at `object-position`; `fill` stretches it to the box
                // as below.
                let style = self.layer_list.layout_tree.render_tree.doc.computed_style(dom_node_id);
                let fit = style.box_group.object_fit;
                if !image_ctx.placeholder && fit != ObjectFit::Fill {
                    let natural = (image_ctx.dimension.width as f32, image_ctx.dimension.height as f32);
                    let position = style.box_group.object_position;
                    let content_box = layout_element.box_model.content_box;
                    if let Some(tiling) = object_fit_placement(natural, content_box, fit, position) {
                        // The rectangle is the whole content box, rounded by its inner radii; the
                        // image is one non-repeating tile in it, and backends paint a tile only
                        // where it lies. So a `contain` image leaves its letterbox empty, a `cover`
                        // one is cropped, and any part of the image under a rounded corner follows
                        // the curve, whether or not the image reaches the other edges.
                        let mut r = Rectangle::new(content_box)
                            .with_background(Brush::image_tiled(image_ctx.media_id, Some(tiling)))
                            .with_blend_mode(blend);
                        let [tl, tr, br, bl] = inner_radii(&style.border, border_box, content_box);
                        if [tl, tr, br, bl].iter().any(|radius| radius.x > 0.0 || radius.y > 0.0) {
                            r = r.with_radius_tlrb(tl, tr, br, bl);
                        }
                        commands.push(PaintCommand::rectangle(r));
                    }
                    if self.has_border(dom_node_id) {
                        let border_r =
                            self.decorate_with_border_and_radius(dom_node_id, None, Rectangle::new(border_box));
                        commands.push(PaintCommand::rectangle(border_r));
                    }
                } else {
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
                    let border_r =
                        self.decorate_with_border_and_radius(dom_node_id, None, Rectangle::new(border_target));
                    if image_ctx.placeholder {
                        commands.push(PaintCommand::rectangle(r));
                        // Emit the element border separately so it frames the full reserved box.
                        if self.has_border(dom_node_id) {
                            commands.push(PaintCommand::rectangle(border_r));
                        }
                    } else {
                        let r = self.decorate_with_border_and_radius(dom_node_id, None, r);
                        commands.push(PaintCommand::rectangle(r));
                    }
                }

                // Browsers show `alt` inside the box when the image renders nothing visible. For a
                // placeholder it sits right of the broken-image icon, else at the box's top-left.
                if let Some(alt) = &image_ctx.alt {
                    let icon_w = if image_ctx.placeholder {
                        image_ctx.dimension.width.min(border_box.width)
                    } else {
                        0.0
                    };
                    if let Some(cmd) = self.image_alt_command(dom_node_id, alt, border_box, icon_w) {
                        commands.push(cmd);
                    }
                }
            }
            ElementContext::FormControl(fc) => {
                // The control's chrome - its background and border - comes first; an inset shadow
                // goes over that and under the value, the caret and the icons.
                let mut control = self.form_control_commands(fc, layout_element, dom_node_id).into_iter();
                commands.extend(control.next());
                commands.extend(inset_shadows.take());
                commands.extend(control);
            }
            ElementContext::SelectPopup(popup) => {
                commands.extend(self.select_popup_commands(popup, layout_element));
            }
            ElementContext::None => {
                let border_box = layout_element.box_model.border_box;
                let (brush, overlay_layers) =
                    self.background_fill(dom_node_id, (border_box.width as f32, border_box.height as f32));
                let r = Rectangle::new(border_box)
                    .with_background(brush)
                    .with_blend_mode(self.mix_blend_mode(dom_node_id));
                // A collapsed cell paints only its background here; its border strips are
                // painted by the table's TableBorderOverlay AFTER all table content.
                let r = if layout_element.collapsed_borders.is_some() {
                    r
                } else {
                    self.decorate_with_border_and_radius(dom_node_id, None, r)
                };
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
            // Collapsed borders of a whole table, painted in front of its content: one
            // border-only rectangle per collapsed cell, in cell paint order.
            ElementContext::TableBorderOverlay(cells) => {
                for &cell_id in cells {
                    let Some(cell) = self.layer_list.layout_tree.get_node_by_id(cell_id) else {
                        continue;
                    };
                    let Some(ref cb) = cell.collapsed_borders else { continue };
                    let r = Rectangle::new(cell.box_model.border_box);
                    let r = self.decorate_with_border_and_radius(cell.dom_node_id, Some(cb), r);
                    commands.push(PaintCommand::rectangle(r));
                }
            }
        }

        commands.extend(inset_shadows);

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

    /// CSS `box-shadow`, as two shadow-only rectangles: the outer shadows, painted before
    /// anything else of the element, and the inset ones, painted over its backgrounds and under
    /// its content. An inset shadow stays inside the padding box, so painting it after the
    /// border does not cover the border.
    fn shadow_commands(
        &self,
        layout_element: &LayoutElementNode,
        dom_node_id: NodeId,
    ) -> (Option<PaintCommand>, Option<PaintCommand>) {
        let doc = &self.layer_list.layout_tree.render_tree.doc;
        let style = doc.computed_style(dom_node_id);
        if style.background.shadows.is_empty() {
            return (None, None);
        }

        let bb = layout_element.box_model.border_box;
        let border = &style.border;
        let radii = used::border_radii(border, bb.width, bb.height);
        let border_box = RoundedRect::new(bb, radii.map(|(x, y)| Radius::new_double(x, y)));

        // The padding box's corners are the border box's, less the border on either side of them.
        let [top, right, bottom, left] = [
            border.top_width,
            border.right_width,
            border.bottom_width,
            border.left_width,
        ]
        .map(f64::from);
        let padding_rect = Rect::new(
            bb.x + left,
            bb.y + top,
            (bb.width - left - right).max(0.0),
            (bb.height - top - bottom).max(0.0),
        );
        let [tl, tr, br, bl] = radii;
        let inner = |(x, y): (f64, f64), side_x: f64, side_y: f64| {
            Radius::new_double((x - side_x).max(0.0), (y - side_y).max(0.0))
        };
        let padding_box = RoundedRect::new(
            padding_rect,
            [
                inner(tl, left, top),
                inner(tr, right, top),
                inner(br, right, bottom),
                inner(bl, left, bottom),
            ],
        );

        let (mut outer, mut inset) = (Vec::new(), Vec::new());
        for shadow in style.background.shadows.iter() {
            let c = shadow.color;
            let Brush::Solid(color) =
                self.apply_opacity(dom_node_id, Brush::solid(Color::from_rgba8(c.r, c.g, c.b, c.a)))
            else {
                continue;
            };
            let offset = (f64::from(shadow.offset_x), f64::from(shadow.offset_y));
            let (blur, spread) = (f64::from(shadow.blur), f64::from(shadow.spread));
            if shadow.inset {
                inset.push(Shadow::inset(padding_box, offset, blur, spread, color));
            } else if let Some(shadow) = Shadow::outer(border_box, offset, blur, spread, color) {
                outer.push(shadow);
            }
        }
        let command = |shadows: Vec<Shadow>| {
            (!shadows.is_empty()).then(|| PaintCommand::rectangle(Rectangle::new(bb).with_shadows(shadows)))
        };
        (command(outer), command(inset))
    }

    /// CSS `outline`: a border-only rectangle around the border box, inflated by offset + width,
    /// following the element's corner radii. Takes no layout space.
    fn outline_command(&self, layout_element: &LayoutElementNode, dom_node_id: NodeId) -> Option<PaintCommand> {
        let doc = &self.layer_list.layout_tree.render_tree.doc;
        let outline = doc.computed_style(dom_node_id).outline.clone();
        let width = f64::from(outline.width);
        if width <= 0.0 {
            return None;
        }
        if !outline.style.is_visible() {
            return None;
        }
        let style = css_border_style_to_paint(&outline.style);
        let offset = f64::from(outline.offset);
        // A negative offset pulls the ring inside the border box.
        let grow = offset + width;

        let bb = layout_element.box_model.border_box;
        let ring = Rect::new(bb.x - grow, bb.y - grow, bb.width + grow * 2.0, bb.height + grow * 2.0);
        if ring.width <= 0.0 || ring.height <= 0.0 {
            return None;
        }

        let brush = self.get_brush(dom_node_id, |style| style.outline.color, Brush::solid(Color::BLACK));
        let border = Border::new(
            width as f32,
            style,
            [brush.clone(), brush.clone(), brush.clone(), brush],
        );
        let mut r = Rectangle::new(ring).with_border(border);

        // The ring follows the border box's corners, each radius grown by the distance the ring
        // sits outside it (css-ui-4 §4.3); a square corner stays square.
        let border = &doc.computed_style(dom_node_id).border;
        let grown = |length: f64| if length > 0.0 { length + grow } else { 0.0 };
        let [tl, tr, br, bl] = used::border_radii(border, bb.width, bb.height).map(|(x, y)| Radius {
            x: grown(x),
            y: grown(y),
        });
        if [tl, tr, br, bl].iter().any(|radius| radius.x > 0.0 || radius.y > 0.0) {
            r = r.with_radius_tlrb(tl, tr, br, bl);
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

        let (bg_brush, _) = self.background_fill(dom_node_id, (border_box.width as f32, border_box.height as f32));
        let chrome = Rectangle::new(border_box)
            .with_background(bg_brush)
            .with_blend_mode(blend);
        let chrome = self.decorate_with_border_and_radius(dom_node_id, None, chrome);
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
            let h = fc.font_info.line_height_px().max(fc.font_info.size);
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
                self.get_brush(dom_node_id, |style| style.inherited.color, Brush::solid(Color::BLACK)),
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
                // A date/time kind holds an ISO string and shows it the way its placeholder
                // promised (`dd-mm-yyyy`); it is picked, not typed, so it has no caret.
                let shown = doc
                    .attribute(dom_node_id, "type")
                    .and_then(|ty| text_field::display_value(&ty, &value));
                let picked = shown.is_some();
                let caret = if picked { None } else { focused.then_some(state.caret) };
                let selection = if focused && !picked { state.selection() } else { None };
                let is_placeholder = value.is_empty();
                let mut text = if is_placeholder {
                    placeholder.clone()
                } else if *masked {
                    "\u{2022}".repeat(value.chars().count())
                } else {
                    shown.unwrap_or_else(|| value.clone())
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
                        content_box.height.max(fc.font_info.line_height_px()),
                    )
                } else {
                    line_rect(inset_x)
                };
                let avail = if *multiline { rect.width } else { 1_000_000_000.0 };
                let line_h = fc.font_info.line_height_px().max(fc.font_info.size);
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
                    .and_then(|s| parse_floating_point_number(&s.value))
                    .map(|v| fraction_between(v, *min, *max).clamp(0.0, 1.0));
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
                // What the picker chose lives on the document, like typed text; the markup
                // value is only the default. Unparsable is black, per value sanitisation.
                let doc = &self.layer_list.layout_tree.render_tree.doc;
                let live = doc.control_edit_state(dom_node_id).map(|s| s.value);
                let color = Color::try_from_css(live.as_deref().unwrap_or(value)).unwrap_or(Color::BLACK);
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
        let border = &doc.computed_style(dom_node_id).border;
        border.top_width != 0.0 || border.right_width != 0.0 || border.bottom_width != 0.0 || border.left_width != 0.0
    }

    /// Apply the element's computed CSS border and border-radius to `r`. Shared by block,
    /// image and SVG elements so replaced elements (`<img>`) get their borders too.
    /// `suppressed` edges (`[top, right, bottom, left]`, from `border-collapse`
    /// conflict resolution) are painted at zero width - the neighbouring cell
    /// paints the shared border instead.
    fn decorate_with_border_and_radius(
        &self,
        dom_node_id: NodeId,
        collapsed: Option<&CollapsedCellBorders>,
        mut r: Rectangle,
    ) -> Rectangle {
        let doc = &self.layer_list.layout_tree.render_tree.doc;

        // Table cells under border-collapse paint the collapse geometry, not
        // their CSS borders: each edge at its layout width (half the resolved
        // boundary - the adjacent cell paints the other half, so the result is
        // independent of cell paint order), lost edges in the winning
        // neighbour's style, and perimeter edges extended outward by the
        // outset (the border covers the background bleed on those strips).
        // border-radius does not apply to collapsed cells.
        if let Some(cb) = collapsed {
            if (0..4).all(|e| cb.widths[e] + cb.outsets[e] <= 0.0) {
                return r;
            }
            let own_color: [ColorOf; 4] = [
                |style| style.border.top_color,
                |style| style.border.right_color,
                |style| style.border.bottom_color,
                |style| style.border.left_color,
            ];
            let own_style: [BorderStyleOf; 4] = [
                |style| style.border.top_style,
                |style| style.border.right_style,
                |style| style.border.bottom_style,
                |style| style.border.left_style,
            ];
            // A lost edge renders the winning neighbour's FACING edge: our top
            // is its bottom, our right is its left, and vice versa. When the winner
            // is the TABLE itself (a perimeter boundary the table's border won),
            // the same side applies - the table's left border faces the same way
            // as the cell's left border.
            let facing_color: [ColorOf; 4] = [
                |style| style.border.bottom_color,
                |style| style.border.left_color,
                |style| style.border.top_color,
                |style| style.border.right_color,
            ];
            let facing_style: [BorderStyleOf; 4] = [
                |style| style.border.bottom_style,
                |style| style.border.left_style,
                |style| style.border.top_style,
                |style| style.border.right_style,
            ];
            let owner_is_table = |owner: NodeId| {
                matches!(
                    doc.computed_style(owner).declared_display(),
                    Some(Display::Table | Display::InlineTable)
                )
            };

            let brushes: [Brush; 4] = std::array::from_fn(|e| match cb.owners[e] {
                Some(owner) if owner_is_table(owner) => self.get_brush(owner, own_color[e], Brush::solid(Color::BLACK)),
                Some(owner) => self.get_brush(owner, facing_color[e], Brush::solid(Color::BLACK)),
                None => self.get_brush(dom_node_id, own_color[e], Brush::solid(Color::BLACK)),
            });
            let styles: [BorderStyle; 4] = std::array::from_fn(|e| {
                let (node, style_of) = match cb.owners[e] {
                    Some(owner) if owner_is_table(owner) => (owner, own_style[e]),
                    Some(owner) => (owner, facing_style[e]),
                    None => (dom_node_id, own_style[e]),
                };
                css_border_style_to_paint(&style_of(&doc.computed_style(node)))
            });
            // Snap the box and the strip ends to whole device pixels. The two
            // cells of a boundary compute their strip ends from the SAME edge
            // coordinate, so they snap identically and the half-strips abut
            // without a double-AA seam - and thin borders land on whole
            // pixels (a fractional 0.5+0.5 split would render as two
            // half-alpha rows instead of one crisp line).
            let rect = r.rect();
            let (x0, y0) = (rect.x, rect.y);
            let (x1, y1) = (rect.x + rect.width, rect.y + rect.height);
            let top = (y0 - cb.outsets[0] as f64).round();
            let right = (x1 + cb.outsets[1] as f64).round();
            let bottom = (y1 + cb.outsets[2] as f64).round();
            let left = (x0 - cb.outsets[3] as f64).round();
            let widths = [
                ((y0 + cb.widths[0] as f64).round() - top) as f32,
                (right - (x1 - cb.widths[1] as f64).round()) as f32,
                (bottom - (y1 - cb.widths[2] as f64).round()) as f32,
                ((x0 + cb.widths[3] as f64).round() - left) as f32,
            ];
            return r
                .with_rect(Rect::new(left, top, right - left, bottom - top))
                .with_border(Border::new_per_side(widths, styles, brushes));
        }

        // A collapsed table's boundary is painted entirely by its perimeter cells
        // (their halves + outsets, in whatever style won the conflict); painting the
        // table's own CSS border as well would double-draw the losing style on top.
        let element_style = doc.computed_style(dom_node_id);
        if matches!(
            element_style.declared_display(),
            Some(Display::Table | Display::InlineTable)
        ) && element_style.inherited.border_collapse == BorderCollapse::Collapse
        {
            return r;
        }

        let css_border = &element_style.border;
        let border_top_width = css_border.top_width;
        let border_right_width = css_border.right_width;
        let border_bottom_width = css_border.bottom_width;
        let border_left_width = css_border.left_width;

        if border_top_width != 0.0
            || border_right_width != 0.0
            || border_bottom_width != 0.0
            || border_left_width != 0.0
        {
            let border_top_color =
                self.get_brush(dom_node_id, |style| style.border.top_color, Brush::solid(Color::BLACK));
            let border_right_color = self.get_brush(
                dom_node_id,
                |style| style.border.right_color,
                Brush::solid(Color::BLACK),
            );
            let border_bottom_color = self.get_brush(
                dom_node_id,
                |style| style.border.bottom_color,
                Brush::solid(Color::BLACK),
            );
            let border_left_color =
                self.get_brush(dom_node_id, |style| style.border.left_color, Brush::solid(Color::BLACK));

            let border = Border::new_per_side(
                [
                    border_top_width,
                    border_right_width,
                    border_bottom_width,
                    border_left_width,
                ],
                [
                    css_border_style_to_paint(&css_border.top_style),
                    css_border_style_to_paint(&css_border.right_style),
                    css_border_style_to_paint(&css_border.bottom_style),
                    css_border_style_to_paint(&css_border.left_style),
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

        let rect = r.rect();
        let [tl, tr, br, bl] = used::border_radii(css_border, rect.width, rect.height).map(|(x, y)| Radius { x, y });
        if [tl, tr, br, bl].iter().any(|radius| radius.x > 0.0 || radius.y > 0.0) {
            r = r.with_radius_tlrb(tl, tr, br, bl);
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

/// Where `object-fit` and `object-position` put an image of `natural` size in `content`: one
/// non-repeating tile, positioned against the content box. It may hang over the box's edges
/// (`cover`, or `none` with a large image); painting is limited to the box and to the tile.
/// `None` when there is no box or no natural size to fit by.
fn object_fit_placement(
    natural: (f32, f32),
    content: Rect,
    fit: ObjectFit,
    position: (Anchor, Anchor),
) -> Option<Tiling> {
    let (nw, nh) = natural;
    let (cw, ch) = (content.width as f32, content.height as f32);
    if nw <= 0.0 || nh <= 0.0 || cw <= 0.0 || ch <= 0.0 {
        return None;
    }
    let contain = (cw / nw).min(ch / nh);
    let (w, h) = match fit {
        ObjectFit::Fill => (cw, ch),
        ObjectFit::Contain => (nw * contain, nh * contain),
        ObjectFit::Cover => {
            let s = (cw / nw).max(ch / nh);
            (nw * s, nh * s)
        }
        ObjectFit::None => (nw, nh),
        ObjectFit::ScaleDown => {
            let s = contain.min(1.0);
            (nw * s, nh * s)
        }
    };
    Some(Tiling {
        tile_size: (w, h),
        position: (anchor_px(position.0, cw, w), anchor_px(position.1, ch, h)),
        repeat: (false, false),
    })
}

/// The corner radii of the content box: the border box's radii less the border and padding on
/// each side, never below zero (css-backgrounds-3 §5.2).
fn inner_radii(border: &gosub_interface::style::BorderGroup, border_box: Rect, content: Rect) -> [Radius; 4] {
    let left = content.x - border_box.x;
    let top = content.y - border_box.y;
    let right = (border_box.x + border_box.width) - (content.x + content.width);
    let bottom = (border_box.y + border_box.height) - (content.y + content.height);
    let [tl, tr, br, bl] = used::border_radii(border, border_box.width, border_box.height);
    let inner = |(x, y): (f64, f64), dx: f64, dy: f64| Radius {
        x: (x - dx).max(0.0),
        y: (y - dy).max(0.0),
    };
    [
        inner(tl, left, top),
        inner(tr, right, top),
        inner(br, right, bottom),
        inner(bl, left, bottom),
    ]
}

/// Where an [`Anchor`] puts the start edge of something `extent` long in a box `box_extent` long.
#[expect(clippy::cast_possible_truncation, reason = "paint geometry is f32")]
fn anchor_px(anchor: Anchor, box_extent: f32, extent: f32) -> f32 {
    used::anchor(anchor, f64::from(box_extent), f64::from(extent)) as f32
}

/// The size `background-size` gives one tile in a positioning area of `area`, for an image of
/// `natural` size, or `None` for a gradient, which has neither a size nor a ratio of its own
/// (css-backgrounds-3 §3.9). An `auto` beside a length keeps the image's ratio; with no ratio to
/// keep it is the area's extent.
#[expect(clippy::cast_possible_truncation, reason = "paint geometry is f32")]
fn used_bg_size(size: BackgroundSize, natural: Option<(f32, f32)>, area: (f32, f32)) -> (f32, f32) {
    let natural = natural.filter(|&(w, h)| w > 0.0 && h > 0.0);
    let scaled = |pick: fn(f32, f32) -> f32| match natural {
        Some((nw, nh)) => {
            let scale = pick(area.0 / nw, area.1 / nh);
            (nw * scale, nh * scale)
        }
        None => area,
    };
    match size {
        BackgroundSize::Contain => scaled(f32::min),
        BackgroundSize::Cover => scaled(f32::max),
        BackgroundSize::Explicit(width, height) => {
            let width = used::length_auto(width, f64::from(area.0)).map(|w| w as f32);
            let height = used::length_auto(height, f64::from(area.1)).map(|h| h as f32);
            match (width, height, natural) {
                (Some(w), Some(h), _) => (w, h),
                (Some(w), None, Some((nw, nh))) => (w, w * nh / nw),
                (None, Some(h), Some((nw, nh))) => (h * nw / nh, h),
                (None, None, Some(natural)) => natural,
                (Some(w), None, None) => (w, area.1),
                (None, Some(h), None) => (area.0, h),
                (None, None, None) => area,
            }
        }
    }
}

/// Places one tile of `tile_size` by a layer's position and repeat, in a box of `area`.
fn tiling(tile: &BackgroundTile, tile_size: (f32, f32), area: (f32, f32)) -> Tiling {
    let repeats = |repeat: BackgroundRepeat| repeat != BackgroundRepeat::NoRepeat;
    Tiling {
        tile_size,
        position: (
            anchor_px(tile.position.0, area.0, tile_size.0),
            anchor_px(tile.position.1, area.1, tile_size.1),
        ),
        repeat: (repeats(tile.repeat.0), repeats(tile.repeat.1)),
    }
}

/// Resolves a raster background layer into a [`Tiling`], now that the border box is known.
/// `space` and `round` tile like `repeat`.
fn compute_bg_tiling(natural: (f32, f32), tile: &BackgroundTile, box_w: f32, box_h: f32) -> Option<Tiling> {
    let (nw, nh) = natural;
    if nw <= 0.0 || nh <= 0.0 || box_w <= 0.0 || box_h <= 0.0 {
        return None;
    }
    let (tw, th) = used_bg_size(tile.size, Some(natural), (box_w, box_h));
    if tw <= 0.0 || th <= 0.0 {
        return None;
    }
    Some(tiling(tile, (tw, th), (box_w, box_h)))
}

/// A gradient layer as the painter draws it: its stops placed along the line, and a tiling unless
/// one tile would be the whole box anyway.
fn gradient_layer(gradient: &CssLinearGradient, tile: &BackgroundTile, area: (f32, f32)) -> Option<Gradient> {
    let stops = gradient_stops(gradient);
    if stops.len() < 2 {
        return None;
    }
    let (tw, th) = used_bg_size(tile.size, None, area);
    let placed = (tw > 0.0 && th > 0.0).then(|| tiling(tile, (tw, th), area));
    let fills_the_box = |t: &Tiling| t.tile_size == area && t.position == (0.0, 0.0);
    Some(Gradient::Linear(LinearGradient {
        angle_deg: gradient.angle_deg,
        stops,
        tiling: placed.filter(|t| !fills_the_box(t)),
    }))
}

/// A gradient's stops with every position settled (css-images-3 §3.5.3): the first at 0 and the
/// last at 1 when left out, the ones between spread evenly across each gap, and none before the
/// one ahead of it.
#[expect(clippy::cast_precision_loss, reason = "a handful of stops")]
fn gradient_stops(gradient: &CssLinearGradient) -> Vec<ColorStop> {
    let mut offsets: Vec<Option<f32>> = gradient.stops.iter().map(|stop| stop.position).collect();
    let n = offsets.len();
    if n < 2 {
        return Vec::new();
    }
    if let Some(first) = offsets.first_mut() {
        first.get_or_insert(0.0);
    }
    if let Some(last) = offsets.last_mut() {
        last.get_or_insert(1.0);
    }
    let mut i = 1;
    while i < n {
        if offsets[i].is_some() {
            i += 1;
            continue;
        }
        let start = i - 1;
        let mut end = i;
        while end < n && offsets[end].is_none() {
            end += 1;
        }
        let a = offsets[start].unwrap_or(0.0);
        let b = offsets.get(end).copied().flatten().unwrap_or(1.0);
        let steps = (end - start) as f32;
        for (k, slot) in offsets.iter_mut().enumerate().take(end).skip(start + 1) {
            *slot = Some(a + (b - a) * ((k - start) as f32) / steps);
        }
        i = end;
    }
    let mut running = 0.0_f32;
    gradient
        .stops
        .iter()
        .zip(offsets)
        .map(|(stop, offset)| {
            let offset = offset.unwrap_or(0.0).clamp(0.0, 1.0).max(running);
            running = offset;
            ColorStop {
                offset,
                color: Color::from_rgba8(stop.color.r, stop.color.g, stop.color.b, stop.color.a),
            }
        })
        .collect()
}

#[cfg(test)]
mod background_size_tests {
    use super::used_bg_size;
    use gosub_interface::style::{BackgroundSize, LengthPercentageAuto as Lpa};

    /// One length is the width, and the height keeps the image's ratio; with no ratio, as for a
    /// gradient, it is the area's height.
    #[test]
    fn an_auto_height_keeps_the_ratio() {
        let size = BackgroundSize::Explicit(Lpa::Px(20.0), Lpa::Auto);
        assert_eq!(used_bg_size(size, Some((100.0, 50.0)), (300.0, 300.0)), (20.0, 10.0));
        assert_eq!(used_bg_size(size, None, (300.0, 120.0)), (20.0, 120.0));
    }

    /// A percentage size is of the positioning area. It used to fall back to filling the box.
    #[test]
    fn a_percentage_size_is_of_the_area() {
        let size = BackgroundSize::Explicit(Lpa::Percent(50.0), Lpa::Percent(50.0));
        assert_eq!(used_bg_size(size, Some((10.0, 10.0)), (200.0, 100.0)), (100.0, 50.0));
    }

    #[test]
    fn cover_and_contain_keep_the_ratio() {
        assert_eq!(
            used_bg_size(BackgroundSize::Contain, Some((100.0, 50.0)), (200.0, 200.0)),
            (200.0, 100.0)
        );
        assert_eq!(
            used_bg_size(BackgroundSize::Cover, Some((100.0, 50.0)), (200.0, 200.0)),
            (400.0, 200.0)
        );
    }
}
