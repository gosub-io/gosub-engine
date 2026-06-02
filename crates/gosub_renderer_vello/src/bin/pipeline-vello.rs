use gosub_render_pipeline::common;
use gosub_render_pipeline::common::browser_state::{BrowserState, WireframeState};
use gosub_render_pipeline::common::document::pipeline_doc::PipelineDocument;
use gosub_render_pipeline::common::geo::{Dimension, Rect};
use gosub_render_pipeline::common::media::MediaStore;
use gosub_render_pipeline::common::TextureStore;
use gosub_render_pipeline::compositor::Composable;
use gosub_render_pipeline::layering::layer::{LayerId, LayerList};
use gosub_render_pipeline::layouter::taffy::TaffyLayouter;
use gosub_render_pipeline::layouter::CanLayout;
use gosub_render_pipeline::painter::Painter;
use gosub_render_pipeline::rasterizer::Rasterable;
use gosub_render_pipeline::render::backends::vello::WgpuResources;
use gosub_render_pipeline::rendertree_builder::RenderTree;
use gosub_render_pipeline::tiler::{TileList, TileState};
use gosub_renderer_vello::compositor::{VelloCompositor, VelloCompositorConfig};
use gosub_renderer_vello::VelloRasterizer;
use parking_lot::{Mutex, RwLock};
use std::cell::RefCell;
use std::fmt::Formatter;
use std::rc::Rc;
use std::sync::Arc;
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

fn main() -> anyhow::Result<()> {
    let doc = common::document::parser::document_from_json("https://brettgfitzgerald.com", "brett.json");

    let window_dimension = Dimension::new(800.0, 600.0);
    let viewport_dimension = Dimension::new(1024.0, 768.0);

    let browser_state = BrowserState {
        visible_layer_list: vec![],
        wireframed: WireframeState::None,
        debug_hover: false,
        current_hovered_element: None,
        show_tilegrid: true,
        viewport: Rect::new(0.0, 0.0, viewport_dimension.width, viewport_dimension.height),
        tile_list: None,
        dpi_scale_factor: 1.0,
    };

    let browser_state = Arc::new(RwLock::new(browser_state));
    let texture_store = Arc::new(RwLock::new(TextureStore::new()));
    let media_store = Arc::new(RwLock::new(MediaStore::new()));
    let doc: Arc<dyn PipelineDocument> = Arc::new(doc);

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = App::new(
        "Pipeline Vello",
        window_dimension,
        doc,
        browser_state,
        texture_store,
        media_store,
    );
    let _ = event_loop.run_app(&mut app);
    Ok(())
}

fn reflow(doc: &Arc<dyn PipelineDocument>, browser_state: &Arc<RwLock<BrowserState>>) {
    let (viewport, dpi_scale_factor) = {
        let state = browser_state.read();
        (state.viewport, state.dpi_scale_factor)
    };

    println!("reflowing to dimension: {:?}", viewport);

    let mut render_tree = RenderTree::new(doc.clone());
    render_tree.parse();

    let mut layouter = TaffyLayouter::new();
    let layout_tree = layouter.layout(
        render_tree,
        Some(Dimension::new(viewport.width, viewport.height)),
        dpi_scale_factor,
    );

    let layer_list = LayerList::new(layout_tree);
    let mut tile_list = TileList::new(layer_list, Dimension::new(TILE_DIMENSION, TILE_DIMENSION));
    tile_list.generate();

    let layer_count = tile_list.layer_list.layer_ids.read().len();
    let mut state = browser_state.write();
    state.visible_layer_list.resize(layer_count, true);
    state.tile_list = Some(RwLock::new(tile_list));
}

#[allow(clippy::arc_with_non_send_sync)]
struct Env<'s> {
    pub render_ctx: RenderContext,
    pub renderer: Option<Rc<RefCell<Renderer>>>,
    pub surface: Option<RenderSurface<'s>>,
    pub window: Option<Arc<Window>>,
}

impl std::fmt::Debug for Env<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Env").finish()
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
    doc: Arc<dyn PipelineDocument>,
    browser_state: Arc<RwLock<BrowserState>>,
    texture_store: Arc<RwLock<TextureStore>>,
    media_store: Arc<RwLock<MediaStore>>,
}

impl App<'_> {
    fn new(
        window_title: &str,
        window_size: Dimension,
        doc: Arc<dyn PipelineDocument>,
        browser_state: Arc<RwLock<BrowserState>>,
        texture_store: Arc<RwLock<TextureStore>>,
        media_store: Arc<RwLock<MediaStore>>,
    ) -> Self {
        App {
            env: None,
            frame: 0,
            pfs: Instant::now(),
            fps: 0.0,
            window_size,
            window_title: window_title.to_string(),
            doc,
            browser_state,
            texture_store,
            media_store,
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
        self.env = match create_window_env(event_loop, self.window_title.as_str(), self.window_size) {
            Ok(env) => Some(env),
            Err(e) => {
                log::error!("Failed to create window: {:?}", e);
                event_loop.exit();
                None
            }
        };

        reflow(&self.doc, &self.browser_state);
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

                let Some(surface) = env.surface.as_mut() else {
                    return;
                };
                env.render_ctx.resize_surface(surface, width, height);

                self.browser_state.write().viewport = Rect::new(0.0, 0.0, width as f64, height as f64);

                reflow(&self.doc, &self.browser_state);
            }
            WindowEvent::RedrawRequested => {
                self.frame += 1;
                self.pfs = Instant::now();
                println!("Redraw requested: framecount: {}", self.frame);

                let Some(surface) = env.surface.as_ref() else {
                    return;
                };
                let dev_id = surface.dev_id;
                let DeviceHandle { device, queue, .. } = &env.render_ctx.devices[dev_id];

                let vis_layers = self.browser_state.read().visible_layer_list.clone();

                let Some(renderer) = env.renderer.as_mut() else {
                    return;
                };

                for (i, &visible) in vis_layers.iter().enumerate() {
                    if visible {
                        do_paint(LayerId::new(i as u64), &self.browser_state);
                        do_rasterize(
                            device,
                            queue,
                            renderer.clone(),
                            LayerId::new(i as u64),
                            &self.browser_state,
                            &self.texture_store,
                            &self.media_store,
                        );
                    }
                }

                let surface_texture = match surface.surface.get_current_texture() {
                    vello::wgpu::CurrentSurfaceTexture::Success(t)
                    | vello::wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
                    _ => {
                        log::error!("Failed to get current texture");
                        return;
                    }
                };

                let render_params = RenderParams {
                    base_color: color::palette::css::DARK_MAGENTA,
                    width: self.browser_state.read().viewport.width as u32,
                    height: self.browser_state.read().viewport.height as u32,
                    antialiasing_method: AaConfig::Msaa16,
                };

                let scene = VelloCompositor::compose(VelloCompositorConfig {
                    browser_state: self.browser_state.clone(),
                    texture_store: self.texture_store.clone(),
                });

                let Some(binding) = env.renderer.clone() else {
                    return;
                };
                let mut renderer = binding.borrow_mut();
                let _ = renderer.render_to_texture(
                    device,
                    queue,
                    &scene,
                    &surface_texture.texture.create_view(&Default::default()),
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
                    let mut state = self.browser_state.write();

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
                    if let Some(v) = state.visible_layer_list.get_mut(layer_id) {
                        *v = !*v;
                    }
                    window.request_redraw();
                }

                if logical_key == "w" {
                    let mut state = self.browser_state.write();

                    match state.wireframed {
                        WireframeState::None => state.wireframed = WireframeState::Only,
                        WireframeState::Only => state.wireframed = WireframeState::Both,
                        WireframeState::Both => state.wireframed = WireframeState::None,
                    }

                    if let Some(ref tile_list) = state.tile_list {
                        tile_list.write().invalidate_all();
                    }
                    window.request_redraw();
                }

                if logical_key == "d" {
                    let mut state = self.browser_state.write();

                    state.debug_hover = !state.debug_hover;

                    if let Some(ref tile_list) = state.tile_list {
                        tile_list.write().invalidate_all();
                    }
                    window.request_redraw();
                }

                if logical_key == "t" {
                    let mut state = self.browser_state.write();
                    state.show_tilegrid = !state.show_tilegrid;
                    drop(state);
                    window.request_redraw();
                }
            }
            _ => (),
        }
    }
}

fn create_window_env<'s>(el: &ActiveEventLoop, title: &str, size: Dimension) -> anyhow::Result<Env<'s>> {
    log::info!(
        "Creating ({}x{}) window with title: {} ",
        size.width,
        size.height,
        title
    );

    let mut render_ctx = RenderContext::new();

    let mut attribs = Window::default_attributes();
    attribs.title = title.to_string();
    attribs.inner_size = Some(Size::Physical(PhysicalSize::new(size.width as u32, size.height as u32)));
    let window = Arc::new(el.create_window(attribs)?);

    let size = window.inner_size();
    let surface_future =
        render_ctx.create_surface(window.clone(), size.width, size.height, wgpu::PresentMode::AutoVsync);
    let surface = pollster::block_on(surface_future)?;

    let dev_handle = &render_ctx.devices[surface.dev_id];

    let renderer = Renderer::new(
        &dev_handle.device,
        RendererOptions {
            use_cpu: false,
            antialiasing_support: AaSupport::all(),
            num_init_threads: None,
            pipeline_cache: None,
        },
    )
    .map_err(|e| anyhow::anyhow!("Failed to create Vello renderer: {:?}", e))?;

    #[allow(clippy::arc_with_non_send_sync)]
    let env = Env {
        render_ctx,
        window: Some(window),
        surface: Some(surface),
        renderer: Some(Rc::new(RefCell::new(renderer))),
    };

    log::info!("vello window created");

    Ok(env)
}

fn do_paint(layer_id: LayerId, browser_state: &Arc<RwLock<BrowserState>>) {
    let state = browser_state.read();

    let Some(ref tile_list) = state.tile_list else {
        log::error!("No tile list found");
        return;
    };

    let painter = Painter::new(tile_list.read().layer_list.clone());

    let tile_ids = tile_list.read().get_intersecting_tiles(layer_id, state.viewport);
    for tile_id in tile_ids {
        let mut binding = tile_list.write();
        let Some(tile) = binding.get_tile_mut(tile_id) else {
            log::warn!("Tile not found: {:?}", tile_id);
            continue;
        };

        if tile.state == TileState::Clean || tile.state == TileState::Empty {
            continue;
        }

        for tiled_layout_element in &mut tile.elements {
            tiled_layout_element.paint_commands = painter.paint(tiled_layout_element, &state);
        }
    }
}

fn do_rasterize(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    _renderer: Rc<RefCell<Renderer>>,
    layer_id: LayerId,
    browser_state: &Arc<RwLock<BrowserState>>,
    texture_store: &Arc<RwLock<TextureStore>>,
    media_store: &Arc<RwLock<MediaStore>>,
) {
    // Build a temporary WgpuResources for the rasterizer. Device and Queue are Clone
    // in wgpu 29 (they hold an Arc internally), so cloning is cheap.
    #[allow(clippy::arc_with_non_send_sync)]
    let state = browser_state.read();
    let mut ts = texture_store.write();
    let ms = media_store.read();

    let Some(ref tile_list) = state.tile_list else {
        log::error!("No tile list found");
        return;
    };

    let tile_ids = tile_list.read().get_intersecting_tiles(layer_id, state.viewport);
    for tile_id in tile_ids {
        let mut binding = tile_list.write();
        let Some(tile) = binding.get_tile(tile_id) else {
            log::warn!("Tile not found: {:?}", tile_id);
            continue;
        };

        if tile.state == TileState::Clean || tile.state == TileState::Empty {
            continue;
        }

        let Some(tile) = binding.get_tile_mut(tile_id) else {
            log::warn!("Tile not found: {:?}", tile_id);
            continue;
        };

        let rasterizer_renderer = Renderer::new(
            device,
            RendererOptions {
                use_cpu: false,
                antialiasing_support: AaSupport::all(),
                num_init_threads: None,
                pipeline_cache: None,
            },
        )
        .unwrap_or_else(|e| panic!("rasterizer Renderer::new failed: {e:?}"));
        let resources = Arc::new(WgpuResources {
            device: Arc::new(device.clone()),
            queue: Arc::new(queue.clone()),
            renderer: Mutex::new(rasterizer_renderer),
        });
        let rasterizer = VelloRasterizer::new(resources);
        match rasterizer.rasterize(tile, &mut ts, &ms) {
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
