use gosub_shared::render_backend::{Point, Radius, RenderBackend, RenderRect, RenderText, Scene as TScene, FP};

use crate::debug::text::render_text_simple;
use crate::{Border, BorderRadius, BorderRenderOptions, CairoBackend, Text, Transform};

pub struct Scene {
    context: cairo::Context,
}

impl Scene {
    pub fn inner(&mut self) -> &mut cairo::Context {
        &mut self.context
    }

    pub fn create(cr: &cairo::Context) -> Self {
        Self {
            context: cr.clone(),
        }
    }
}

impl TScene<CairoBackend> for Scene {
    fn draw_rect(&mut self, rect: &RenderRect<CairoBackend>) {
        // let affine = rect.transform.as_ref().map(|t| t.0).unwrap_or_default();

        let brush = &rect.brush;
        // let brush_transform = rect.brush_transform.as_ref().map(|t| t.0);

        if let Some(radius) = &rect.radius {
            self.context.set_source_rgba(brush.r, brush.g, brush.b, brush.a);
            draw_rounded_rect(&mut self.context, rect.rect.x as FP, rect.rect.y as FP, rect.rect.width as FP, rect.rect.height as FP, radius);
        } else {
            self.context.set_source_rgba(brush.r, brush.g, brush.b, brush.a);
            self.context.rectangle(rect.rect.x, rect.rect.y, rect.rect.width, rect.rect.height);
        }

        if let Some(border) = &rect.border {
            let opts = BorderRenderOptions {
                border,
                rect: &rect.rect,
                transform: rect.transform.as_ref(),
                radius: rect.radius.as_ref(),
            };

            Border::draw(&mut self.context, opts);
        }
    }

    fn draw_text(&mut self, text: &RenderText<CairoBackend>) {
        Text::show(&mut self.context, text)
    }

    fn debug_draw_simple_text(&mut self, text: &str, pos: Point, size: FP) {
        render_text_simple(self, text, pos, size)
    }

    fn apply_scene(&mut self, _scene: &<CairoBackend as RenderBackend>::Scene, _transform: Option<Transform>) {
        // There is nothing to apply, as all operations are done on the context immediately
    }

    fn reset(&mut self) {
        // There is nothing to reset. All operations are done on the context immediately
    }

    fn new() -> Self {
        CairoBackend::new().into()
    }
}


/// Draws a rounded rectangle with specified border radii.
pub fn draw_rounded_rect(
    cr: &cairo::Context,
    x: FP,
    y: FP,
    width: FP,
    height: FP,
    radius: &BorderRadius,
) {
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
