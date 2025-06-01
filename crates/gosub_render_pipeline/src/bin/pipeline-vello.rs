#[cfg(not(feature = "backend_vello"))]
compile_error!("This binary can only be used with the feature 'backend_vello' enabled");

use poc_pipeline::common;
use poc_pipeline::common::browser_state::{
    get_browser_state, init_browser_state, BrowserState, WireframeState,
};
use poc_pipeline::common::geo::{Dimension, Rect};
use poc_pipeline::compositor::vello::{VelloCompositor, VelloCompositorConfig};
use poc_pipeline::compositor::Composable;
use poc_pipeline::layering::layer::{LayerId, LayerList};
use poc_pipeline::layouter::taffy::TaffyLayouter;
use poc_pipeline::layouter::CanLayout;
use poc_pipeline::painter::Painter;
use poc_pipeline::rasterizer::vello::VelloRasterizer;
use poc_pipeline::rasterizer::Rasterable;
use poc_pipeline::rendertree_builder::RenderTree;
use poc_pipeline::tiler::{TileList, TileState};
use std::cell::RefCell;
use std::fmt::Formatter;
use std::sync::Arc;
use std::sync::RwLock;
use std::time::Instant;
use vello::peniko::color;
use vello::util::{DeviceHandle, RenderContext, RenderSurface};
use vello::{wgpu, AaConfig, AaSupport, RenderParams, Renderer, RendererOptions};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalSize, Size};
use winit::event::{KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::KeyCode;
use winit::keyboard::PhysicalKey::Code;
use winit::window::{Window, WindowId};

const TILE_DIMENSION: f64 = 256.0;

fn main() {
    // --------------------------------------------------------------------
    // Generate a DOM tree
    // let doc = common::document::parser::document_from_json("https://codemusings.nl", "cm.json");
    let doc = common::document::parser::document_from_json("https://brettgfitzgerald.com", "brett.json");

    let window_dimension = Dimension::new(800.0, 600.0);
    let viewport_dimension = Dimension::new(1024.0, 768.0);

    let browser_state = BrowserState {
        // visible_layer_list: vec![true; 10],
        visible_layer_list: vec![true, false, false, false, false, false, false, false, false, false],
        wireframed: WireframeState::None,
        debug_hover: false,
        current_hovered_element: None,
        show_tilegrid: true,
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

    let mut app = App::new("Pipeline Vello", window_dimension);
    let _ = event_loop.run_app(&mut app);
}

fn reflow() {
    let binding = get_browser_state();
    let state = binding.read().unwrap();

    println!("reflowing to dimension: {:?}", state.viewport);

    let mut render_tree = RenderTree::new(state.document.clone());
    render_tree.parse();

    let mut layouter = TaffyLayouter::new();
    let layout_tree = layouter.layout(
        render_tree,
        Some(Dimension::new(state.viewport.width, state.viewport.height)),
        state.dpi_scale_factor,
    );

    let layer_list = LayerList::new(layout_tree);

    let mut tile_list = TileList::new(layer_list, Dimension::new(TILE_DIMENSION, TILE_DIMENSION));
    tile_list.generate();

    drop(state);

    let binding = get_browser_state();
    let mut state = binding.write().unwrap();
    state.tile_list = Some(RwLock::new(tile_list));
}

struct Env<'s> {
    pub render_ctx: RenderContext,
    pub renderer: Option<Arc<RefCell<Renderer>>>,
    pub surface: Option<RenderSurface<'s>>, // Surface must be before window for safety during cleanup
    pub window: Option<Arc<Window>>,
}

impl std::fmt::Debug for Env<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Env")
            .finish()
    }
}

struct App<'s> {
    env: Option<Env<'s>>,
    frame: usize,
    pfs: Instant,
    #[allow(unused)]
    fps: f32,
    window_size: Dimension,
    window_title: String,
}

impl App<'_> {
    fn new(window_title: &str, window_size: Dimension) -> Self {
        App {
            env: None,
            frame: 0,
            pfs: Instant::now(),
            fps: 0.0,
            window_size,
            window_title: window_title.to_string(),
        }
    }
}

impl ApplicationHandler for App<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.env.is_some() {
            return;
        }

        self.pfs = Instant::now();
        self.frame = 0;
        self.env = Some(create_window_env(
            event_loop,
            self.window_title.as_str(),
            self.window_size
        ));

        reflow();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(env) = &mut self.env else { return };

        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::Resized(physical_size) => {
                println!("Resized to {:?}", physical_size);

                let (width, height): (u32, u32) = physical_size.into();

                env.render_ctx.resize_surface(
                    &mut env.surface.as_mut().unwrap(),
                    width,
                    height,
                );

                let binding = get_browser_state();
                let mut state = binding.write().unwrap();
                state.viewport = Rect::new(0.0, 0.0, width as f64, height as f64);
                drop(state);

                // reflow();
            }
            WindowEvent::RedrawRequested => {
                self.frame += 1;
                self.pfs = Instant::now();
                println!("Redraw requested: framecount: {}", self.frame);

                let surface = env.surface.as_ref().unwrap();
                let dev_id = surface.dev_id;
                let DeviceHandle { device, queue, .. } = &env.render_ctx.devices[dev_id];

                let binding = get_browser_state();
                let state = binding.read().unwrap();
                let vis_layers = state.visible_layer_list.clone();
                drop(state);

                let renderer = &mut env.renderer.as_mut().unwrap();

                for i in 0..10 {
                    if vis_layers[i] {
                        do_paint(LayerId::new(i as u64));
                        do_rasterize(device, queue, renderer.clone(), LayerId::new(i as u64));
                    }
                }

                let surface_texture = surface
                    .surface
                    .get_current_texture()
                    .expect("Failed to get current texture");


                let binding = get_browser_state();
                let state = binding.read().unwrap();

                let render_params = RenderParams {
                    base_color: color::palette::css::DARK_MAGENTA,
                    width: state.viewport.width as u32,
                    height: state.viewport.height as u32,
                    // width: self.window_size.width as u32,
                    // height: self.window_size.height as u32,
                    antialiasing_method: AaConfig::Msaa16,
                };

                let scene = VelloCompositor::compose(VelloCompositorConfig {});

                let binding = env.renderer.clone().unwrap();
                let mut renderer = binding.borrow_mut();
                let _ = renderer.render_to_surface(
                    device,
                    queue,
                    &scene,
                    &surface_texture,
                    &render_params,
                );

                surface_texture.present();
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key,
                        logical_key,
                        state,
                        repeat,
                        ..
                    },
                ..
            } => {
                if !state.is_pressed() || repeat {
                    return;
                }

                let Some(window) = env.window.as_ref() else {
                    return;
                };

                if logical_key == "q" {
                    event_loop.exit();
                }

                if physical_key >= Code(KeyCode::Digit0) && physical_key <= Code(KeyCode::Digit9) {
                    let binding = get_browser_state();
                    let mut state = binding.write().unwrap();

                    let layer_id = match physical_key {
                        Code(KeyCode::Digit1) => 0,
                        Code(KeyCode::Digit2) => 1,
                        Code(KeyCode::Digit3) => 2,
                        Code(KeyCode::Digit4) => 3,
                        Code(KeyCode::Digit5) => 4,
                        Code(KeyCode::Digit6) => 5,
                        Code(KeyCode::Digit7) => 6,
                        Code(KeyCode::Digit8) => 7,
                        Code(KeyCode::Digit9) => 8,
                        Code(KeyCode::Digit0) => 9,
                        _ => unreachable!(),
                    };
                    state.visible_layer_list[layer_id] = !state.visible_layer_list[layer_id];
                    window.request_redraw();
                }

                if logical_key == "w" {
                    let binding = get_browser_state();
                    let mut state = binding.write().unwrap();

                    match state.wireframed {
                        WireframeState::None => state.wireframed = WireframeState::Only,
                        WireframeState::Only => state.wireframed = WireframeState::Both,
                        WireframeState::Both => state.wireframed = WireframeState::None,
                    }

                    let Some(ref tile_list) = state.tile_list else {
                        log::error!("No tile list found");
                        return;
                    };

                    tile_list
                        .write()
                        .expect("Failed to get tile list")
                        .invalidate_all();
                    window.request_redraw();
                }

                if logical_key == "d" {
                    let binding = get_browser_state();
                    let mut state = binding.write().unwrap();

                    state.debug_hover = !state.debug_hover;
                    window.request_redraw();
                }

                if logical_key == "t" {
                    let binding = get_browser_state();
                    let mut state = binding.write().unwrap();

                    state.show_tilegrid = !state.show_tilegrid;
                    window.request_redraw();
                }

            }
            _ => (),
        }
    }
}

fn create_window_env<'s>(el: &ActiveEventLoop, title: &str, size: Dimension) -> Env<'s> {
    log::info!(
        "Creating ({}x{}) window with title: {} ",
        size.width, size.height, title
    );

    let mut render_ctx = RenderContext::new();

    let mut attribs = Window::default_attributes();
    attribs.title = title.to_string();
    attribs.inner_size = Some(Size::Physical(PhysicalSize::new(size.width as u32, size.height as u32)));
    let window = Arc::new(el.create_window(attribs).unwrap());

    let size = window.inner_size();
    let surface_future = render_ctx.create_surface(
        window.clone(),
        size.width,
        size.height,
        wgpu::PresentMode::AutoVsync,
    );
    let surface = pollster::block_on(surface_future).expect("Failed to create surface");

    let dev_handle = &render_ctx.devices[surface.dev_id];

    let renderer = Renderer::new(
        &dev_handle.device,
        RendererOptions {
            surface_format: Some(surface.format),
            use_cpu: false,
            antialiasing_support: AaSupport::all(),
            num_init_threads: None,
        },
    );

    let env = Env {
        render_ctx,
        window: Some(window),
        surface: Some(surface),
        renderer: Some(Arc::new(RefCell::new(renderer.unwrap()))),
    };

    log::info!("vello window created");

    env
}


fn do_paint(layer_id: LayerId) {
    let binding = get_browser_state();
    let state = binding.read().unwrap();

    let Some(ref tile_list) = state.tile_list else {
        log::error!("No tile list found");
        return;
    };

    let painter = Painter::new(tile_list.read().unwrap().layer_list.clone());

    let tile_ids = tile_list
        .read()
        .unwrap()
        .get_intersecting_tiles(layer_id, state.viewport);
    for tile_id in tile_ids {
        // get tile
        let mut binding = tile_list.write().expect("Failed to get tile list");
        let Some(tile) = binding.get_tile_mut(tile_id) else {
            log::warn!("Tile not found: {:?}", tile_id);
            continue;
        };

        // if not dirty, no need to render and continue
        if tile.state == TileState::Clean || tile.state == TileState::Empty {
            continue;
        }

        // Paint all the elements in each tile
        for tiled_layout_element in &mut tile.elements {
            tiled_layout_element.paint_commands = painter.paint(tiled_layout_element);
        }
    }
}

fn do_rasterize(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: Arc<RefCell<Renderer>>,
    layer_id: LayerId,
) {
    let binding = get_browser_state();
    let state = binding.read().unwrap();

    let Some(ref tile_list) = state.tile_list else {
        log::error!("No tile list found");
        return;
    };

    let tile_ids = tile_list
        .read()
        .unwrap()
        .get_intersecting_tiles(layer_id, state.viewport);
    for tile_id in tile_ids {
        // get tile
        let mut binding = tile_list.write().expect("Failed to get tile list");
        let Some(tile) = binding.get_tile(tile_id) else {
            log::warn!("Tile not found: {:?}", tile_id);
            continue;
        };

        // if not dirty, no need to render and continue
        if tile.state == TileState::Clean || tile.state == TileState::Empty {
            continue;
        }


        let Some(tile) = binding.get_tile_mut(tile_id) else {
            log::warn!("Tile not found: {:?}", tile_id);
            continue;
        };

        // Rasterize the tile into a texture
        let rasterizer = VelloRasterizer::new(device, queue, &renderer);
        match rasterizer.rasterize(tile) {
            Some(texture_id) => {
                tile.texture_id = Some(texture_id);
                tile.state = TileState::Clean;
            }
            None => {
                tile.state = TileState::Empty;
            }
        }
    }
}
