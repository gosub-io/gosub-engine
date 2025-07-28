use futures::channel::mpsc;
use futures::channel::mpsc::UnboundedSender;
use futures::executor::block_on;
use futures::StreamExt;
use gosub_cairo::{CairoBackend, Scene};
use gosub_css3::system::Css3System;
use gosub_fontmanager::FontManager;
use gosub_html5::document::builder::DocumentBuilderImpl;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::document::fragment::DocumentFragmentImpl;
use gosub_html5::parser::Html5Parser;
use gosub_instance::{EngineInstance, InstanceMessage};
use gosub_interface::chrome::ChromeHandle;
use gosub_interface::config::{
    HasChrome, HasCssSystem, HasDocument, HasHtmlParser, HasLayouter, HasRenderBackend, HasRenderTree, HasTreeDrawer,
    ModuleConfiguration,
};
use gosub_interface::font::HasFontManager;
use gosub_interface::instance::{Handles, InstanceId};
use gosub_interface::render_backend::RenderBackend;
use gosub_interface::render_backend::SizeU32;
use gosub_interface::request::RequestServerHandle;
use gosub_renderer::draw::TreeDrawerImpl;
use gosub_rendering::render_tree::RenderTree;
use gosub_taffy::TaffyLayouter;
use gtk4::gio::{ApplicationCommandLine, ApplicationFlags};
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

impl HasChrome for Config {
    type ChromeHandle = GtkChromeHandle;
}

impl HasFontManager for Config {
    type FontManager = FontManager;
}

impl ModuleConfiguration for Config {}

#[derive(Clone)]
struct GtkChromeHandle(UnboundedSender<Scene>);

impl ChromeHandle<Config> for GtkChromeHandle {
    fn draw_scene(&self, scene: <CairoBackend as RenderBackend>::Scene, _: SizeU32, _: InstanceId) {
        info!("Drawing scene");
        self.0.unbounded_send(scene).unwrap();
    }
}

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
    let url = args
        .get(1)
        .and_then(|url| url.to_str())
        .unwrap_or("https://gosub.io/tests/gopher.html")
        .to_string();

    // Create a window and set the title
    let window = ApplicationWindow::builder()
        .application(app)
        .title("GTK Renderer")
        .build();

    let (tx, rx) = mpsc::unbounded();

    let rx = Arc::new(Mutex::new(rx));

    let handles = Handles::<Config> {
        chrome: GtkChromeHandle(tx),
        request: RequestServerHandle,
    };

    // Tree drawer that will render the final tree
    let instance =
        EngineInstance::new_on_thread(Url::parse(&url).unwrap(), TaffyLayouter, InstanceId(0), handles).unwrap();

    // Set up drawing area widget with a custom draw function. This will render a scene using the
    // tree drawer.
    let area = DrawingArea::default();

    area.set_draw_func(move |_area, cr, width, height| {
        let size = SizeU32::new(width as u32, height as u32);

        let tx = instance.tx.clone();
        let cr = cr.clone();
        let rx = rx.clone();

        #[allow(clippy::await_holding_lock)]
        // we will always able to lock the mutex, since we are the only one holding it (stupid futures_channel)
        //TODO: we need a better way than blocking the draw call
        // the problem is that rendering is done by another thread but we can only finish the frame when we have the scene
        // ideally we can have some system that is able to request a redraw from the instance in the `DrawingArea`'s draw_func, and then it returns
        // afterwards if we got the finished scene we can ask for the DrawingArea to be redrawn again and this time we ONLY render the available scene,
        // and DO NOT request another frame from the instance
        block_on(async move {
            tx.send(InstanceMessage::Redraw(size)).await.unwrap();

            let scene = rx.lock().unwrap().next().await;

            if let Some(scene) = scene {
                info!("Rendering scene to context");
                scene.render_to_context(&cr);
            }
        });
    });

    let scroll = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Automatic)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .child(&area)
        .build();
    window.set_child(Some(&scroll));

    window.set_default_width(800);
    window.set_default_height(600);
    window.present();

    1
}
