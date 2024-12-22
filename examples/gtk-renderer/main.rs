use gosub_cairo::render::window::{ActiveWindowData, WindowData};
use gosub_cairo::{CairoBackend, Scene};
use gosub_css3::system::Css3System;
use gosub_html5::document::builder::DocumentBuilderImpl;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::document::fragment::DocumentFragmentImpl;
use gosub_html5::parser::Html5Parser;
use gosub_interface::config::*;
use gosub_interface::draw::TreeDrawer;
use gosub_interface::render_backend::RenderBackend;
use gosub_interface::render_backend::{ImageBuffer, SizeU32, WindowedEventLoop};
use gosub_renderer::draw::TreeDrawerImpl;
use gosub_rendering::render_tree::RenderTree;
use gosub_taffy::TaffyLayouter;
use gtk4::gio::{ApplicationCommandLine, ApplicationFlags};
use gtk4::glib::spawn_future_local;
use gtk4::prelude::*;
use gtk4::{glib, Application, ApplicationWindow, DrawingArea};
use log::{info, LevelFilter};
use simple_logger::SimpleLogger;
use std::sync::{Arc, Mutex};
use url::Url;

const APP_ID: &str = "io.gosub.gtk-renderer";

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

impl HasHtmlParser for Config {
    type HtmlParser = Html5Parser<'static, Self>;
}

impl HasLayouter for Config {
    type Layouter = TaffyLayouter;
    type LayoutTree = RenderTree<Self>;
}

impl HasRenderTree for Config {
    type RenderTree = RenderTree<Self>;
}

impl HasTreeDrawer for Config {
    type TreeDrawer = TreeDrawerImpl<Self>;
}

impl HasRenderBackend for Config {
    type RenderBackend = CairoBackend;
}

impl ModuleConfiguration for Config {}

fn main() -> glib::ExitCode {
    SimpleLogger::new().with_level(LevelFilter::Info).init().unwrap();

    let app = Application::builder()
        .application_id(APP_ID)
        .flags(ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    app.connect_command_line(build_ui);

    app.run()
}

fn build_ui(app: &Application, cl: &ApplicationCommandLine) -> i32 {
    let binding = cl.arguments();
    let args = binding.as_slice();
    let url = args[1].to_str().unwrap_or("https://example.com").to_string();

    // Create a window and set the title
    let window = ApplicationWindow::builder()
        .application(app)
        .title("GTK Renderer")
        .build();

    // Tree drawer that will render the final tree
    let drawer = Arc::new(Mutex::new(Option::<<Config as HasTreeDrawer>::TreeDrawer>::None));

    // Set up drawing area widget with a custom draw function. This will render a scene using the
    // tree drawer.
    let area = DrawingArea::default();
    area.set_draw_func(move |area, cr, width, height| {
        let mut drawer_lock = drawer.lock().unwrap();

        // If there is a drawer initialized, just draw the tree onto the cairo context (cr)
        if let Some(drawer) = drawer_lock.as_mut() {
            let mut render_backend = <Config as HasRenderBackend>::RenderBackend::new();
            let size = SizeU32::new(width as u32, height as u32);

            // Drawer.draw will populate the scene with elements from the tree
            let mut win_data = WindowData {
                scene: Scene::new(),
                cr: Some(cr.clone()),
            };
            drawer.draw(&mut render_backend, &mut win_data, size, &WindowEventLoopDummy);

            // Render the scene to the cairo context
            let mut active_win_data = ActiveWindowData { cr: cr.clone() };
            _ = render_backend.render(&mut win_data, &mut active_win_data);

            return;
        }
        drop(drawer_lock);

        // Drawer does not exist yet. Create it from within a new async task, and trigger a redraw
        // once it's ready.

        let url = url.clone();
        let area = area.clone();
        let drawer = drawer.clone();
        spawn_future_local(async move {
            let d = <Config as HasTreeDrawer>::TreeDrawer::from_url(Url::parse(&url).unwrap(), TaffyLayouter, false)
                .await
                .unwrap();

            let mut drawer_lock = drawer.lock().unwrap();
            *drawer_lock = Some(d);
            drop(drawer_lock);

            area.queue_draw();
        });
    });

    window.set_default_width(800);
    window.set_default_height(600);
    window.set_child(Some(&area));
    window.present();

    1
}

#[derive(Clone)]
struct WindowEventLoopDummy;

impl WindowedEventLoop<Config> for WindowEventLoopDummy {
    fn redraw(&mut self) {
        info!("eventloop: Redraw needed");
    }

    fn add_img_cache(
        &mut self,
        url: String,
        _buf: ImageBuffer<<Config as HasRenderBackend>::RenderBackend>,
        _size: Option<SizeU32>,
    ) {
        info!("eventloop: Add image to cache: {}", url);
    }

    fn reload_from(&mut self, _rt: <Config as HasRenderTree>::RenderTree) {
        info!("eventloop: reload from")
    }

    fn open_tab(&mut self, _url: Url) {
        info!("eventloop: open tab")
    }
}
