use std::fs;
use std::str::FromStr;
use std::sync::{Arc, RwLock};
use clap::ArgAction;
use glutin::context::PossiblyCurrentContext;
use glutin::surface::WindowSurface;
use skia_safe::{
    gpu::{self, gl::FramebufferInfo},
    Surface,
};
use log::LevelFilter;
use simple_logger::SimpleLogger;
use url::Url;
use winit::event_loop::{ControlFlow, EventLoop};
use winit::window::Window;
use gosub_html5::document::builder::DocumentBuilderImpl;
use gosub_interface::document::DocumentBuilder;
use gosub_render_pipeline::common::browser_state::{get_browser_state, init_browser_state, BrowserState, WireframeState};
use gosub_render_pipeline::common::geo::{Dimension, Rect};
use gosub_render_pipeline::layouter::CanLayout;
use gosub_render_pipeline::layouter::taffy::TaffyLayouter;
use gosub_render_pipeline::rendertree_builder::RenderTree;
use gosub_shared::byte_stream::{ByteStream, Encoding};
use glutin::{
    surface::{Surface as GlutinSurface},
};

const TILE_DIMENSION: f64 = 256.0;

mod config;
mod app;
mod renderer;

use app::App;
use gosub_html5::parser::Html5Parser;
use gosub_render_pipeline::layering::layer::LayerList;
use gosub_render_pipeline::tiler::TileList;
use crate::config::Config;

fn main() -> anyhow::Result<()> {
    SimpleLogger::new().with_level(LevelFilter::Info).init().unwrap();

    let matches = clap::Command::new("Gosub Skia Renderer Example")
        .arg(
            clap::Arg::new("url")
                .help("The url or file to parse")
                .required(true)
                .index(1),
        )
        .arg(
            clap::Arg::new("debug")
                .short('d')
                .long("debug")
                .action(ArgAction::SetTrue),
        )
        .get_matches();

    let url: String = matches.get_one::<String>("url").expect("url").to_string();
    let url = Url::from_str(&url).unwrap_or_else(|_| panic!("Invalid url"));

    let html = if url.scheme() == "http" || url.scheme() == "https" {
        // Fetch the html from the url
        let Ok(mut response) = ureq::get(url.as_ref()).call() else {
            panic!("Could not get url.");
        };
        if response.status() != 200 {
            panic!("Could not get url. Status code {}", response.status());
        }
        response.body_mut().read_to_string()
            .unwrap_or_else(|_| panic!("Could not read response body"))
    } else if url.scheme() == "file" {
        // Get html from the file
        fs::read_to_string(url.to_string().trim_start_matches("file://"))
            .unwrap_or_else(|_| panic!("Could not read file: {}", url.path()))
    } else {
        panic!("Invalid url scheme");
    };

    let mut stream = ByteStream::new(Encoding::UTF8, None);
    stream.read_from_str(&html, Some(Encoding::UTF8));
    stream.close();

    // Create a new document that will be filled in by the parser
    let mut doc = <DocumentBuilderImpl as DocumentBuilder<Config>>::new_document(Some(url));
    let parse_errors = Html5Parser::<Config>::parse_document(&mut stream, &mut doc, None)?;

    // let mut output = String::new();
    // doc.print_tree(&mut output);
    // println!("{}", output);

    let window_dimension = Dimension::new(800.0, 600.0);
    let viewport_dimension = Dimension::new(1280.0, 1144.0);

    let browser_state = BrowserState {
        visible_layer_list: vec![true; 10],
        wireframed: WireframeState::None,
        debug_hover: false,
        current_hovered_element: None,
        show_tilegrid: false,
        viewport: Rect::new(
            0.0,
            0.0,
            viewport_dimension.width,
            viewport_dimension.height,
        ),
        document: Arc::new(doc),
        tile_list: None,
        dpi_scale_factor: 1.0,
    };
    init_browser_state(browser_state);

    let event_loop = EventLoop::new().unwrap();
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = App::new("Skia Pipeline Test", window_dimension);
    let _ = event_loop.run_app(&mut app);

    Ok(())
}


fn reflow() {
    let binding = get_browser_state();
    let state = binding.read().unwrap();

    let mut render_tree = RenderTree::new(state.document.clone());
    render_tree.parse();

    let mut layouter = TaffyLayouter::new();
    let layout_tree = layouter.layout(
        render_tree,
        Some(Dimension::new(state.viewport.width, state.viewport.height)),
        state.dpi_scale_factor,
    );
    // layouter.print_tree();

    let layer_list = LayerList::new(layout_tree);

    let mut tile_list = TileList::new(layer_list, Dimension::new(TILE_DIMENSION, TILE_DIMENSION));
    tile_list.generate();

    // tile_list.print_list();

    drop(state);

    let binding = get_browser_state();
    let mut state = binding.write().unwrap();
    state.tile_list = Some(RwLock::new(tile_list));
}


struct Env {
    pub surface: Surface,
    pub gl_surface: GlutinSurface<WindowSurface>,
    pub gr_context: gpu::DirectContext,
    pub gl_context: PossiblyCurrentContext,
    pub window: Window,

    pub fb_info: FramebufferInfo,
    pub num_samples: usize,
    pub stencil_size: usize,
}

impl std::fmt::Debug for Env {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Env")
            .field("fb_info", &self.fb_info)
            .field("num_samples", &self.num_samples)
            .field("stencil_size", &self.stencil_size)
            .finish()
    }
}

