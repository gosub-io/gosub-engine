use std::fmt::{Debug, Formatter};
use std::sync::{mpsc, Arc, Mutex};
use gosub_shared::render_backend::{Point, Radius, RenderBackend, RenderRect, RenderText, Scene as TScene, FP};

use crate::debug::text::render_text_simple;
use crate::{BorderRadius, CairoBackend, Text, Transform};

pub struct CairoRenderContext {
    sender: mpsc::Sender<Box<dyn FnOnce(&cairo::Context) + Send>>,
}

impl CairoRenderContext {
    pub fn new(context: cairo::Context) -> Self {
        let (sender, receiver) = mpsc::channel::<Box<dyn FnOnce(&cairo::Context) + Send>>();

        std::thread::spawn(move || {
            while let Ok(task) = receiver.recv() {
                task(&context);
            }
        });

        Self { sender }
    }

    pub fn render<F>(&self, func: F)
    where
        F: FnOnce(&cairo::Context) + Send + 'static,
    {
        self.sender.send(Box::new(func)).unwrap();
    }
}


pub struct Scene {
    pub(crate) crc: CairoRenderContext,
}

impl Scene{
    pub fn inner(&mut self) -> &mut Arc<Mutex<cairo::Context>> {
        &mut self.context
    }

    pub fn create(cr: &cairo::Context) -> Self {
        Self {
            crc: CairoRenderContext::new(cr.clone()),
        }
    }
}

impl Clone for Scene {
    fn clone(&self) -> Self {
        todo!()
    }
}

impl Debug for Scene {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scene").finish()
    }
}

impl TScene<CairoBackend> for Scene {
    fn draw_rect(&mut self, rect: &RenderRect<CairoBackend>) {

        self.crc.render(|cr| {
            cr.rectangle(rect.rect.x, rect.rect.y, rect.rect.width, rect.rect.height);
        });

        // // let affine = rect.transform.as_ref().map(|t| t.0).unwrap_or_default();
        //
        // let brush = &rect.brush.brush;
        // // let brush_transform = rect.brush_transform.as_ref().map(|t| t.0);
        // match brush {
        //     CairoBrush::Gradient(gradient) => {
        //         gradient.apply(&mut self.context, rect.rect.x as FP, rect.rect.y as FP, rect.rect.width as FP, rect.rect.height as FP);
        //     }
        //     CairoBrush::Solid(color) => {
        //         self.context.set_source_rgba(color.r(), color.g(), color.b(), color.a());
        //         draw_rounded_rect(&mut self.context, rect.rect.x as FP, rect.rect.y as FP, rect.rect.width as FP, rect.rect.height as FP, radius);
        //     }
        //     CairoBrush::Image(image) => {
        //         let surface = image.surface();
        //         self.context.set_source_surface(&surface, rect.rect.x as f64, rect.rect.y as f64);
        //         self.context.paint();
        //     }
        // }
        //
        //
        // if let Some(radius) = &rect.radius {
        //     draw_rounded_rect(&mut self.context, rect.rect.x as FP, rect.rect.y as FP, rect.rect.width as FP, rect.rect.height as FP, radius);
        // } else {
        //
        //     self.context.set_source_rgba(brush.r, brush.g, brush.b, brush.a);
        //     self.context.rectangle(rect.rect.x, rect.rect.y, rect.rect.width, rect.rect.height);
        // }
        //
        // if let Some(border) = &rect.border {
        //     let opts = BorderRenderOptions {
        //         border,
        //         rect: &rect.rect,
        //         transform: rect.transform.as_ref(),
        //         radius: rect.radius.as_ref(),
        //     };
        //
        //     Border::draw(&mut self.context, opts);
        // }
    }

    fn draw_text(&mut self, text: &RenderText<CairoBackend>) {
        Text::show(self, text)
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
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 800, 600).unwrap();
        let context = cairo::Context::new(&surface).unwrap();

        Self {
            crc: CairoRenderContext::new(context),
        }
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
