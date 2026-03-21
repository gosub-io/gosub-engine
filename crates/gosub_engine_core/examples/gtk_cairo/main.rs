//! gtk_cairo — GTK4 + CairoBackend example
//!
//! Opens a GTK4 window and renders a static scene using the engine's
//! CairoBackend.  Navigation is not wired up here — the scene is built
//! manually to demonstrate the render pipeline independently of the layout
//! bridge (which is not yet implemented).
//!
//! Run:
//!   cargo run --example gtk_cairo --features gtk4,backend_cairo

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, RwLock};

use gtk4::cairo::Context;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{Application, ApplicationWindow, DrawingArea};

use gosub_engine_core::render::backend::{PresentMode, RenderBackend, SurfaceSize};
use gosub_engine_core::render::backends::cairo::{CairoBackend, CairoSurface};
use gosub_engine_core::render::backend::RenderContext;
use gosub_engine_core::render::{Color, DisplayItem, RenderList, Viewport};

const APP_ID: &str = "io.gosub.gtk-cairo-example";

// ---------------------------------------------------------------------------
// A minimal RenderContext impl — holds viewport + render list.
// In the future this will be driven by the layout bridge.
// ---------------------------------------------------------------------------

struct SimpleRenderContext {
    viewport: Viewport,
    render_list: RenderList,
}

impl SimpleRenderContext {
    fn new(width: u32, height: u32) -> Self {
        let mut rl = RenderList::new();

        // White background
        rl.add_command(DisplayItem::Clear { color: Color::WHITE });

        // A blue banner
        rl.add_command(DisplayItem::Rect {
            x: 0.0,
            y: 0.0,
            w: width as f32,
            h: 60.0,
            color: Color::new(0.13, 0.36, 0.78, 1.0),
        });

        // Title text
        rl.add_command(DisplayItem::TextRun {
            x: 20.0,
            y: 42.0,
            text: "Gosub Engine — gtk_cairo example".into(),
            size: 24.0,
            color: Color::WHITE,
            max_width: None,
        });

        // Body text
        rl.add_command(DisplayItem::TextRun {
            x: 20.0,
            y: 100.0,
            text: "Render pipeline is working. Layout bridge coming soon.".into(),
            size: 16.0,
            color: Color::BLACK,
            max_width: Some(width as f32 - 40.0),
        });

        // A demo rectangle
        rl.add_command(DisplayItem::Rect {
            x: 20.0,
            y: 130.0,
            w: 200.0,
            h: 80.0,
            color: Color::new(0.8, 0.2, 0.2, 1.0),
        });
        rl.add_command(DisplayItem::Rect {
            x: 240.0,
            y: 130.0,
            w: 200.0,
            h: 80.0,
            color: Color::new(0.2, 0.7, 0.2, 1.0),
        });
        rl.add_command(DisplayItem::Rect {
            x: 460.0,
            y: 130.0,
            w: 200.0,
            h: 80.0,
            color: Color::new(0.2, 0.2, 0.8, 1.0),
        });

        Self {
            viewport: Viewport::new(0, 0, width, height),
            render_list: rl,
        }
    }
}

impl RenderContext for SimpleRenderContext {
    fn viewport(&self) -> &Viewport {
        &self.viewport
    }
    fn render_list(&self) -> &RenderList {
        &self.render_list
    }
}

// ---------------------------------------------------------------------------
// GTK glue
// ---------------------------------------------------------------------------

fn build_ui(app: &Application) {
    let width = 800u32;
    let height = 600u32;

    let backend = CairoBackend::new();
    let mut surface = backend
        .create_surface(SurfaceSize { width, height }, PresentMode::Immediate)
        .expect("failed to create CairoSurface");

    // Render the static scene once
    let mut ctx = SimpleRenderContext::new(width, height);
    backend.render(&mut ctx, &mut *surface).expect("render failed");

    // Copy pixels out for GTK painting
    let cairo_surface = surface
        .as_any_mut()
        .downcast_mut::<CairoSurface>()
        .expect("expected CairoSurface");

    let (pixels, w, h, stride) = cairo_surface.pixels_borrowed();
    // Clone the pixel data so we can move it into the drawing callback
    let pixel_data: Vec<u8> = pixels.to_vec();
    let pixel_data = Rc::new(RefCell::new((pixel_data, w, h, stride)));

    let drawing_area = DrawingArea::builder()
        .content_width(width as i32)
        .content_height(height as i32)
        .build();

    let pixel_data_clone = pixel_data.clone();
    drawing_area.set_draw_func(move |_area, cr: &Context, _w, _h| {
        let (data, pw, ph, stride) = &*pixel_data_clone.borrow();

        // Create a Cairo ImageSurface from our pixel buffer (ARGB32)
        let img = gtk4::cairo::ImageSurface::create_for_data(
            data.clone(),
            gtk4::cairo::Format::ARgb32,
            *pw as i32,
            *ph as i32,
            *stride as i32,
        )
        .expect("failed to create image surface");

        cr.set_source_surface(&img, 0.0, 0.0).unwrap();
        cr.paint().unwrap();
    });

    let window = ApplicationWindow::builder()
        .application(app)
        .title("Gosub — gtk_cairo example")
        .default_width(width as i32)
        .default_height(height as i32)
        .child(&drawing_area)
        .build();

    window.present();
}

fn main() -> glib::ExitCode {
    let app = Application::builder().application_id(APP_ID).build();
    app.connect_activate(build_ui);
    app.run()
}
