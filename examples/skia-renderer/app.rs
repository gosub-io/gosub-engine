use std::ffi::CString;
use std::num::NonZeroU32;
use std::time::Instant;
use gl_rs::types::GLint;
use glutin::config::ConfigTemplateBuilder;
use glutin::context::{ContextApi, ContextAttributesBuilder};
use glutin::display::GetGlDisplay;
use glutin::prelude::*;
use glutin::surface::{SurfaceAttributesBuilder, WindowSurface};
use glutin_winit::DisplayBuilder;
use log::info;
use raw_window_handle::HasWindowHandle;
use skia_safe::{gpu, ColorType, Surface};
use skia_safe::gpu::{backend_render_targets, SurfaceOrigin};
use skia_safe::gpu::gl::FramebufferInfo;
use winit::application::ApplicationHandler;
use winit::event::{KeyEvent, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::KeyCode;
use winit::keyboard::PhysicalKey::Code;
use winit::window::{Window, WindowAttributes, WindowId};
use gosub_render_pipeline::common::browser_state::{get_browser_state, WireframeState};
use gosub_render_pipeline::common::geo::{Dimension, Rect};
use gosub_render_pipeline::compositor::Composable;
use gosub_render_pipeline::compositor::skia::{SkiaCompositor, SkiaCompositorConfig};
use gosub_render_pipeline::layering::layer::LayerId;
use crate::{reflow, Env};
use crate::renderer::{do_paint, do_rasterize};

pub struct App {
    /// Opengl stuff
    env: Option<Env>,
    /// Current frame nr
    frame: usize,
    /// Previous frame start time
    pfs: Instant,
    /// Current FPS
    #[allow(unused)]
    fps: f32,
    ///
    window_size: Dimension,
    window_title: String,
}

impl App {
    pub fn new(window_title: &str, window_size: Dimension) -> Self {
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

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.env.is_some() {
            return;
        }

        self.pfs = Instant::now();
        self.frame = 0;
        self.env = Some(create_window_env(
            event_loop,
            &self.window_title,
            self.window_size,
        ));
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(env) = &mut self.env else { return };

        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                let binding = get_browser_state();
                let state = binding.read().unwrap();
                println!("Scale factor changed from {} to {}", state.dpi_scale_factor, scale_factor);
                drop(state);

                let mut state = binding.write().unwrap();
                state.dpi_scale_factor = scale_factor as f32;
                drop(state);

                env.window.request_redraw();
            }
            WindowEvent::Resized(physical_size) => {
                println!("Resized to {:?}", physical_size);
                env.surface = create_surface(
                    &env.window,
                    env.fb_info,
                    &mut env.gr_context,
                    env.num_samples,
                    env.stencil_size,
                );

                let (width, height): (u32, u32) = physical_size.into();

                env.gl_surface.resize(
                    &env.gl_context,
                    NonZeroU32::new(width.max(1)).unwrap(),
                    NonZeroU32::new(height.max(1)).unwrap(),
                );

                let binding = get_browser_state();
                let mut state = binding.write().unwrap();
                state.viewport = Rect::new(0.0, 0.0, width as f64, height as f64);
                drop(state);

                reflow();
            }
            WindowEvent::RedrawRequested => {
                self.frame += 1;

                // This is wrong
                // self.fps = 1.0 / self.pfs.elapsed().as_secs_f32();
                self.pfs = Instant::now();
                // println!("FPS: {:.2}", self.fps);

                let canvas = env.surface.canvas();
                canvas.clear(skia_safe::Color::WHITE);

                let binding = get_browser_state();
                let state = binding.read().unwrap();
                let vis_layers = state.visible_layer_list.clone();
                drop(state);

                for i in 0..10 {
                    if vis_layers[i] {
                        do_paint(LayerId::new(i as u64));
                        do_rasterize(LayerId::new(i as u64));
                    }
                }

                let canvas = env.surface.canvas();
                let _surface = SkiaCompositor::compose(SkiaCompositorConfig { canvas });

                env.gr_context.flush_and_submit();
                env.gl_surface.swap_buffers(&env.gl_context).unwrap();
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
                    env.window.request_redraw();
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
                    env.window.request_redraw();
                }

                if logical_key == "d" {
                    let binding = get_browser_state();
                    let mut state = binding.write().unwrap();

                    state.debug_hover = !state.debug_hover;
                    env.window.request_redraw();
                }

                if logical_key == "t" {
                    let binding = get_browser_state();
                    let mut state = binding.write().unwrap();

                    state.show_tilegrid = !state.show_tilegrid;
                    env.window.request_redraw();
                }
            }
            _ => (),
        }
    }
}

fn create_window_env(el: &ActiveEventLoop, title: &str, size: Dimension) -> Env {
    info!(
        "Creating ({}x{}) window with title: {} ",
        size.width, size.height, title
    );

    // --------------------------------------------------------------------
    // Initialize Skia/OpenGl stuff
    let window_attrs = WindowAttributes::default()
        .with_title(title)
        .with_inner_size(winit::dpi::PhysicalSize::new(size.width, size.height));

    let template = ConfigTemplateBuilder::new()
        .with_alpha_size(8)
        .with_transparency(true);

    let display_builder = DisplayBuilder::new().with_window_attributes(Some(window_attrs));
    let (window, gl_config) = display_builder
        .build(el, template, |configs| {
            configs
                .reduce(|accum, config| {
                    let transparency_check = config.supports_transparency().unwrap_or(false)
                        & !accum.supports_transparency().unwrap_or(false);

                    if transparency_check || config.num_samples() < accum.num_samples() {
                        config
                    } else {
                        accum
                    }
                })
                .unwrap()
        })
        .unwrap();

    let window = window.expect("Failed to create window with OpenGL context");
    let raw_window_handle = window.window_handle().unwrap();

    let context_attributes = ContextAttributesBuilder::new().build(Some(raw_window_handle.into()));

    let fallback_context_attributes = ContextAttributesBuilder::new()
        .with_context_api(ContextApi::Gles(None))
        .build(Some(raw_window_handle.into()));

    let not_current_gl_context = unsafe {
        gl_config
            .display()
            .create_context(&gl_config, &context_attributes)
            .unwrap_or_else(|_| {
                gl_config
                    .display()
                    .create_context(&gl_config, &fallback_context_attributes)
                    .expect("Failed to create GL context")
            })
    };

    let (width, height) = window.inner_size().into();

    let attrs = SurfaceAttributesBuilder::<WindowSurface>::new().build(
        raw_window_handle.into(),
        NonZeroU32::new(width).unwrap(),
        NonZeroU32::new(height).unwrap(),
    );

    let gl_surface = unsafe {
        gl_config
            .display()
            .create_window_surface(&gl_config, &attrs)
            .expect("Failed to create GL surface")
    };

    let gl_context = not_current_gl_context
        .make_current(&gl_surface)
        .expect("Failed to make GL context current");

    gl_rs::load_with(|s| {
        gl_config
            .display()
            .get_proc_address(CString::new(s).unwrap().as_c_str())
    });
    let interface = gpu::gl::Interface::new_load_with(|name| {
        if name == "eglGetCurrentDisplay" {
            return std::ptr::null();
        }
        gl_config
            .display()
            .get_proc_address(CString::new(name).unwrap().as_c_str())
    })
        .expect("Could not create interface");

    #[allow(deprecated)]
    let mut gr_context =
        gpu::DirectContext::new_gl(interface, None).expect("Failed to create GPU context for Skia");

    let fb_info = {
        let mut fboid: GLint = 0;
        unsafe { gl_rs::GetIntegerv(gl_rs::FRAMEBUFFER_BINDING, &mut fboid) };

        FramebufferInfo {
            fboid: fboid.try_into().unwrap(),
            format: gpu::gl::Format::RGBA8.into(),
            ..Default::default()
        }
    };

    let num_samples = gl_config.num_samples() as usize;
    let stencil_size = gl_config.stencil_size() as usize;

    let surface = create_surface(&window, fb_info, &mut gr_context, num_samples, stencil_size);

    let env = Env {
        surface,
        gl_surface,
        gl_context,
        gr_context,
        window,
        fb_info,
        num_samples,
        stencil_size,
    };

    info!("OpenGL window created");

    env
}

fn create_surface(
    window: &Window,
    fb_info: FramebufferInfo,
    gr_context: &mut gpu::DirectContext,
    num_samples: usize,
    stencil_size: usize,
) -> Surface {
    info!("Creating OpenGL/Skia surface");

    let size = window.inner_size();
    let size = (
        size.width.try_into().expect("Failed to convert width"),
        size.height.try_into().expect("Failed to convert height"),
    );
    info!("Size: {:?}", size);
    let backend_render_target =
        backend_render_targets::make_gl(size, num_samples, stencil_size, fb_info);

    gpu::surfaces::wrap_backend_render_target(
        gr_context,
        &backend_render_target,
        SurfaceOrigin::BottomLeft,
        ColorType::RGBA8888,
        None,
        None,
    ).expect("Failed to create surface")
}

