use crate::elements::border::GsBorderRadius;
use crate::elements::rect::GsRect;
use crate::elements::text::GsText;
use crate::elements::transform::GsTransform;
use crate::CairoBackend;
use gosub_interface::font::FontBlob;
use gosub_interface::render_backend::{
    Point, Radius, RenderBackend, RenderRect, RenderText, Scene as TScene, Transform as TTransform, FP,
};
use std::fmt::{Debug, Formatter};

/// A scene command that can be executed onto a cairo context.
#[derive(Clone)]
pub enum SceneCommand {
    // Draw a rectangle, including rounded corners and border
    Rectangle(Box<RenderRect<CairoBackend>>),
    // Draw a text
    Text(Box<RenderText<CairoBackend>>),
    // Draw a simple text without too much decoration and in a single font / color
    SimpleText {
        text: String,
        font: FontBlob,
        pos: Point,
    },
    // Group a list of commands together on a certain transform (translation, rotation, scale)
    Group {
        children: Vec<SceneCommand>,
        transform: GsTransform,
    },
}

impl SceneCommand {
    fn new_group() -> SceneCommand {
        SceneCommand::Group {
            children: vec![],
            transform: TTransform::IDENTITY,
        }
    }

    fn simple_text(_text: String, _pos: Point, _size: FP) -> SceneCommand {
        // let mut f = GsRenderFont::default();
        // f.set_size(size as f32);

        todo!()

        // SceneCommand::SimpleText {
        //     text,
        //     font: f,
        //     pos,
        // }
    }
}

impl Debug for SceneCommand {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            SceneCommand::Rectangle(rect) => f.debug_struct("Rectangle").field("rect", &rect).finish(),
            SceneCommand::Text(text) => f.debug_struct("Text").field("text", &text).finish(),
            SceneCommand::Group { children, transform } => f
                .debug_struct("Group")
                .field("children", &children)
                .field("transform", &transform)
                .finish(),
            SceneCommand::SimpleText { text, font, pos } => f
                .debug_struct("SimpleText")
                .field("text", text)
                .field("font", font)
                .field("pos", pos)
                .finish(),
        }
    }
}

/// A scene holds all the commands that must be executed onto the given cairo context.
#[derive(Debug)]
pub struct Scene {
    /// Root node is always a `SceneCommand::Group()`
    pub(crate) root: SceneCommand,
}

impl Default for Scene {
    fn default() -> Self {
        Self::new()
    }
}

impl Scene {
    #[must_use]
    pub fn new() -> Self {
        Self {
            root: SceneCommand::new_group(),
        }
    }

    pub fn render_to_context(&self, cr: &cairo::Context) {
        Self::render_scene_command(&self.root, cr);
    }

    fn render_scene_command(scene_command: &SceneCommand, cr: &cairo::Context) {
        match scene_command {
            SceneCommand::Group { children, .. } => {
                // let affine = transform.map(|t| t.0).unwrap_or_default();
                // cr.transform(affine);
                for child in children {
                    Self::render_scene_command(child, cr);
                }
            }
            SceneCommand::Rectangle(rect) => {
                GsRect::render(rect, cr);
            }
            SceneCommand::Text(text) => {
                GsText::render(text, cr);
            }
            SceneCommand::SimpleText {
                text: _,
                font: _,
                pos: _,
            } => {
                // let pango_ctx = pangocairo::functions::create_context(cr);
                // let layout = Layout::new(&pango_ctx);
                //
                // let font_desc = font.get_font_description();
                // layout.set_font_description(Some(&font_desc));
                // layout.set_text(text);
                //
                // cr.move_to(pos.x.into(), pos.y.into());
                // cr.set_source_rgb(0.0, 0.0, 1.0);
                // pangocairo::functions::show_layout(cr, &layout);
            }
        }
    }
}

impl Clone for Scene {
    fn clone(&self) -> Self {
        Self {
            root: self.root.clone(),
        }
    }
}

impl TScene<CairoBackend> for Scene {
    fn draw_rect(&mut self, rect: &RenderRect<CairoBackend>) {
        if let SceneCommand::Group { ref mut children, .. } = &mut self.root {
            children.push(SceneCommand::Rectangle(Box::new(rect.clone())));
        }
    }

    fn draw_text(&mut self, text: &RenderText<CairoBackend>) {
        if let SceneCommand::Group { ref mut children, .. } = &mut self.root {
            children.push(SceneCommand::Text(Box::new(text.clone())));
        }
    }

    fn debug_draw_simple_text(&mut self, text: &str, pos: Point, size: FP) {
        if let SceneCommand::Group { ref mut children, .. } = &mut self.root {
            children.push(SceneCommand::simple_text(text.to_string(), pos, size));
        }
    }

    fn apply_scene(&mut self, scene: &<CairoBackend as RenderBackend>::Scene, transform: Option<GsTransform>) {
        if let SceneCommand::Group { ref mut children, .. } = &mut self.root {
            children.push(SceneCommand::Group {
                children: vec![scene.root.clone()],
                transform: transform.unwrap_or(GsTransform::IDENTITY),
            });
        }
    }

    fn reset(&mut self) {
        self.root = SceneCommand::new_group();
    }

    fn new() -> Self {
        Self {
            root: SceneCommand::new_group(),
        }
    }
}

/// Draws a rounded rectangle with specified border radii.
pub fn draw_rounded_rect(cr: &cairo::Context, x: FP, y: FP, width: FP, height: FP, radius: &GsBorderRadius) {
    // Helper function to get radius dimensions
    let extract_radius = |r: &Radius| match r {
        Radius::Uniform(r) => (*r, *r),
        Radius::Elliptical(rx, ry) => (*rx, *ry),
    };

    let (tl_rx, tl_ry) = extract_radius(&radius.top_left);
    let (tr_rx, tr_ry) = extract_radius(&radius.top_right);
    let (bl_rx, bl_ry) = extract_radius(&radius.bottom_left);
    let (br_rx, br_ry) = extract_radius(&radius.bottom_right);

    // Start in the top-left corner, adjusted for the top-left radius
    cr.move_to(f64::from(x + tl_rx), f64::from(y));

    // Top edge and top-right corner
    cr.line_to(f64::from(x + width - tr_rx), f64::from(y));
    if tr_rx > 0.0 && tr_ry > 0.0 {
        cr.arc(
            f64::from(x + width - tr_rx),
            f64::from(y + tr_ry),
            f64::from(tr_rx.min(tr_ry)),
            -std::f64::consts::FRAC_PI_2,
            0.0,
        );
    }

    // Right edge and bottom-right corner
    cr.line_to(f64::from(x + width), f64::from(y + height - br_ry));
    if br_rx > 0.0 && br_ry > 0.0 {
        cr.arc(
            f64::from(x + width - br_rx),
            f64::from(y + height - br_ry),
            f64::from(br_rx.min(br_ry)),
            0.0,
            std::f64::consts::FRAC_PI_2,
        );
    }

    // Bottom edge and bottom-left corner
    cr.line_to(f64::from(x + bl_rx), f64::from(y + height));
    if bl_rx > 0.0 && bl_ry > 0.0 {
        cr.arc(
            f64::from(x + bl_rx),
            f64::from(y + height - bl_ry),
            f64::from(bl_rx.min(bl_ry)),
            std::f64::consts::FRAC_PI_2,
            std::f64::consts::PI,
        );
    }

    // Left edge and top-left corner
    cr.line_to(f64::from(x), f64::from(y + tl_ry));
    if tl_rx > 0.0 && tl_ry > 0.0 {
        cr.arc(
            f64::from(x + tl_rx),
            f64::from(y + tl_ry),
            f64::from(tl_rx.min(tl_ry)),
            std::f64::consts::PI,
            3.0 * std::f64::consts::FRAC_PI_2,
        );
    }

    cr.close_path();
}
