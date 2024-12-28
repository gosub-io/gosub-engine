use crate::elements::border::GsBorderRadius;
use crate::elements::rect::GsRect;
use crate::elements::text::GsText;
use crate::elements::transform::GsTransform;
use crate::CairoBackend;
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
        pos: Point,
        size: FP,
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

    fn simple_text(text: String, pos: Point, size: FP) -> SceneCommand {
        SceneCommand::SimpleText { text, pos, size }
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
            SceneCommand::SimpleText { text, pos, size } => f
                .debug_struct("SimpleText")
                .field("text", text)
                .field("pos", pos)
                .field("size", size)
                .finish(),
        }
    }
}

/// A scene holds all the commands that must be executed onto the given cairo context.
#[derive(Debug)]
pub struct Scene {
    /// Root node is always a SceneCommand::Group()
    pub(crate) root: SceneCommand,
}

impl Default for Scene {
    fn default() -> Self {
        Self::new()
    }
}

impl Scene {
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
            SceneCommand::SimpleText { text, pos, size } => {
                let face =
                    &cairo::FontFace::toy_create("sans-serif", cairo::FontSlant::Normal, cairo::FontWeight::Bold)
                        .unwrap();
                cr.set_font_face(face);
                let fs: f32 = *size;
                cr.set_font_size(fs as f64);

                cr.move_to(pos.x.into(), pos.y.into());
                cr.set_source_rgb(0.0, 0.0, 1.0);

                _ = cr.show_text(text);
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
    cr.move_to((x + tl_rx) as f64, y as f64);

    // Top edge and top-right corner
    cr.line_to((x + width - tr_rx) as f64, y as f64);
    if tr_rx > 0.0 && tr_ry > 0.0 {
        cr.arc(
            (x + width - tr_rx) as f64,
            (y + tr_ry) as f64,
            tr_rx.min(tr_ry) as f64,
            -std::f64::consts::FRAC_PI_2,
            0.0,
        );
    }

    // Right edge and bottom-right corner
    cr.line_to((x + width) as f64, (y + height - br_ry) as f64);
    if br_rx > 0.0 && br_ry > 0.0 {
        cr.arc(
            (x + width - br_rx) as f64,
            (y + height - br_ry) as f64,
            br_rx.min(br_ry) as f64,
            0.0,
            std::f64::consts::FRAC_PI_2,
        );
    }

    // Bottom edge and bottom-left corner
    cr.line_to((x + bl_rx) as f64, (y + height) as f64);
    if bl_rx > 0.0 && bl_ry > 0.0 {
        cr.arc(
            (x + bl_rx) as f64,
            (y + height - bl_ry) as f64,
            bl_rx.min(bl_ry) as f64,
            std::f64::consts::FRAC_PI_2,
            std::f64::consts::PI,
        );
    }

    // Left edge and top-left corner
    cr.line_to(x as f64, (y + tl_ry) as f64);
    if tl_rx > 0.0 && tl_ry > 0.0 {
        cr.arc(
            (x + tl_rx) as f64,
            (y + tl_ry) as f64,
            tl_rx.min(tl_ry) as f64,
            std::f64::consts::PI,
            3.0 * std::f64::consts::FRAC_PI_2,
        );
    }

    cr.close_path();
}
