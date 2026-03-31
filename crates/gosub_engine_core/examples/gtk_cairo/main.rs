//! gtk_cairo — GTK4 + CairoBackend + full rendering pipeline
//!
//! Fetches a URL (default: https://example.com), parses it with the HTML5
//! parser, runs it through the gosub_pipeline (layout → layers → tiles →
//! paint), and displays the result in a GTK4 window using the CairoBackend.
//!
//! Run:
//!   cargo run --example gtk_cairo --features gtk4,backend_cairo -- https://gosub.io

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gtk4::cairo::Context;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::gio::ApplicationFlags;
use gtk4::{Application, ApplicationWindow, DrawingArea, ScrolledWindow};

use gosub_css3::system::Css3System;
use gosub_engine_core::BrowsingContext;
use gosub_engine_core::render::backend::{PresentMode, RenderBackend, SurfaceSize};
use gosub_engine_core::render::backends::cairo::{CairoBackend, CairoSurface};
use gosub_engine_core::render::Viewport;
use gosub_html5::document::builder::DocumentBuilderImpl;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::document::fragment::DocumentFragmentImpl;
use gosub_html5::parser::Html5Parser;
use gosub_interface::config::{HasCssSystem, HasDocument};
use gosub_interface::css3::CssSystem;
use gosub_interface::document::{Document, DocumentBuilder};
use gosub_stream::byte_stream::{ByteStream, Encoding};

const APP_ID: &str = "io.gosub.gtk-cairo-example";

// ---------------------------------------------------------------------------
// Config wires together the concrete HTML5 / CSS3 implementations.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
struct Config;

impl HasCssSystem for Config {
    type CssSystem = Css3System;
}

impl HasDocument for Config {
    type Document = DocumentImpl<Self>;
    type DocumentFragment = DocumentFragmentImpl<Self>;
    type DocumentBuilder = DocumentBuilderImpl;
}

// ---------------------------------------------------------------------------
// Fetch, parse and pipeline-render a URL into an ARGB pixel buffer.
// Returns (pixels, width, height, stride).
// ---------------------------------------------------------------------------

fn render_url(url: &str, vp_width: u32, vp_height: u32) -> (Vec<u8>, u32, u32, u32) {
    // 1. Fetch HTML
    let html = reqwest::blocking::get(url)
        .unwrap_or_else(|e| panic!("fetch {url}: {e}"))
        .text()
        .expect("decode response body");

    // 2. Parse into a gosub document
    let parsed_url = reqwest::Url::parse(url).ok();
    let mut gosub_doc =
        <DocumentBuilderImpl as DocumentBuilder<Config>>::new_document(parsed_url);
    gosub_doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());

    let mut stream = ByteStream::new(Encoding::UTF8, None);
    stream.read_from_str(&html, Some(Encoding::UTF8));
    stream.close();
    let _ = Html5Parser::<Config>::parse_document(&mut stream, &mut gosub_doc, None);

    // 3. Run through BrowsingContext (pipeline → DisplayItems)
    let mut ctx = BrowsingContext::<Config>::new();
    ctx.set_document(Arc::new(gosub_doc));
    ctx.set_viewport(Viewport::new(0, 0, vp_width, vp_height));
    ctx.rebuild_render_list_if_needed();

    // 4. Render DisplayItems → Cairo pixel buffer
    let backend = CairoBackend::new();
    let mut surface = backend
        .create_surface(
            SurfaceSize { width: vp_width, height: vp_height },
            PresentMode::Immediate,
        )
        .expect("failed to create CairoSurface");

    backend.render(&mut ctx, &mut *surface).expect("render failed");

    let cairo_surface = surface
        .as_any_mut()
        .downcast_mut::<CairoSurface>()
        .expect("expected CairoSurface");

    let (pixels, w, h, stride) = cairo_surface.pixels_borrowed();
    (pixels.to_vec(), w, h, stride)
}

// ---------------------------------------------------------------------------
// GTK glue
// ---------------------------------------------------------------------------

fn build_ui(app: &Application, rendered: (Vec<u8>, u32, u32, u32), url: String) {
    let (pixel_data, pw, ph, stride) = rendered;
    println!("Displaying {pw}×{ph} px");

    let pixel_data = Rc::new(RefCell::new((pixel_data, pw, ph, stride)));

    let drawing_area = DrawingArea::builder()
        .content_width(pw as i32)
        .content_height(ph as i32)
        .build();

    let pixel_data_clone = pixel_data.clone();
    drawing_area.set_draw_func(move |_area, cr: &Context, _w, _h| {
        let (data, pw, ph, stride) = &*pixel_data_clone.borrow();

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

    let scroll = ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Automatic)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .child(&drawing_area)
        .build();

    let window = ApplicationWindow::builder()
        .application(app)
        .title(format!("Gosub — {url}"))
        .default_width(1024)
        .default_height(768)
        .child(&scroll)
        .build();

    window.present();
}

fn main() -> glib::ExitCode {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "https://example.com".to_string());

    println!("Fetching and rendering {url} …");
    let rendered = render_url(&url, 1280, 900);
    println!("Rendered {}×{} px", rendered.1, rendered.2);

    let app = Application::builder()
        .application_id(APP_ID)
        .flags(ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_activate(move |app| build_ui(app, rendered.clone(), url.clone()));
    app.run_with_args(&[] as &[&str])
}
