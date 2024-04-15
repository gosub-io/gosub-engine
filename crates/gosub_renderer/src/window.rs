use std::cell::RefCell;
use std::num::NonZeroUsize;
use std::rc::Rc;
use std::sync::Arc;

use anyhow::anyhow;
use vello::peniko::Color;
use vello::util::{RenderContext, RenderSurface};
use vello::{AaConfig, AaSupport, RenderParams, Renderer, RendererOptions, Scene};
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{Event, WindowEvent};
use winit::event_loop;
use winit::event_loop::EventLoopWindowTarget;
use winit::window::{Window, WindowBuilder, WindowId};

use gosub_shared::types::Result;

pub enum RenderState<'a> {
    Active {
        surface: RenderSurface<'a>,
        window: Arc<Window>,
    },
    Suspended(Arc<Window>),
}

type RenderFn<'a, T> = dyn FnMut(&mut Scene, PhysicalSize<u32>, &mut T) -> bool + 'a;
type CustomEvent = ();
type EventLoop = event_loop::EventLoop<CustomEvent>;
type WindowEventLoop = EventLoopWindowTarget<CustomEvent>;

pub struct WindowState<'a, T> {
    event_loop: EventLoop,
    internal: WindowStateInternal<'a, T>,
}

struct WindowStateInternal<'a, T> {
    render_state: RenderState<'a>,
    renderer: Box<RenderFn<'a, T>>,
    renderer_state: T,
    cx: RenderContext,
    renderers: Vec<Option<Renderer>>,
    scene: Scene,
}

impl<'a, T> WindowState<'a, T> {
    pub fn new(render_fn: Box<RenderFn<'a, T>>, state: T) -> Result<Self> {
        let event_loop = EventLoop::new()?;
        let render_state = RenderState::Suspended(create_window(&event_loop)?);
        let cx = RenderContext::new().map_err(|e| anyhow!(e.to_string()))?;

        Ok(Self {
            event_loop,
            internal: WindowStateInternal {
                render_state,
                renderer: render_fn,
                renderer_state: state,
                cx,
                scene: Scene::new(),
                renderers: Vec::new(),
            },
        })
    }

    pub fn start(mut self) -> Result<()> {
        let error = Rc::new(RefCell::new(None));

        let event_loop_error = error.clone();

        let int = &mut self.internal;
        self.event_loop.run(move |event, event_loop| match event {
            Event::Resumed => {
                let RenderState::Suspended(window) = &int.render_state else {
                    return;
                };

                let size = window.inner_size();

                let surface_future = int.cx.create_surface(
                    window.clone(),
                    size.width,
                    size.height,
                    wgpu::PresentMode::AutoVsync,
                );

                let surface =
                    futures::executor::block_on(surface_future).expect("Failed to create surface");

                int.renderers.resize_with(int.cx.devices.len(), || None);
                let id = surface.dev_id;
                let options = RendererOptions {
                    surface_format: Some(surface.format),
                    use_cpu: false,
                    antialiasing_support: AaSupport::all(),
                    num_init_threads: NonZeroUsize::new(4),
                };

                let Ok(renderer) = Renderer::new(&int.cx.devices[id].device, options) else {
                    *event_loop_error.borrow_mut() = Some(anyhow!("Failed to create renderer"));
                    event_loop.exit();
                    return;
                };
                int.renderers[id] = Some(renderer);

                int.render_state = RenderState::Active {
                    surface,
                    window: window.clone(),
                };

                event_loop.set_control_flow(event_loop::ControlFlow::Poll);
            }
            Event::Suspended => {
                if let RenderState::Active { window, .. } = &int.render_state {
                    int.render_state = RenderState::Suspended(window.clone());
                };

                event_loop.set_control_flow(event_loop::ControlFlow::Wait);
            }

            Event::AboutToWait => {
                if let RenderState::Active { window, .. } = &int.render_state {
                    window.request_redraw();
                }
            }
            Event::WindowEvent { event, window_id } => {
                if let Err(e) = int.handle_window_event(event, window_id, event_loop) {
                    *event_loop_error.borrow_mut() = Some(e);
                    event_loop.exit();
                };
            }
            _ => {}
        })?;

        match Rc::try_unwrap(error) {
            Ok(error) => {
                if let Some(error) = error.into_inner() {
                    return Err(error);
                }
            }
            Err(e) => {
                if let Some(e) = e.borrow().as_ref() {
                    return Err(anyhow!(e.to_string()));
                };
            }
        }

        Ok(())
    }
}

impl<'a, T> WindowStateInternal<'a, T> {
    fn handle_window_event(
        &mut self,
        event: WindowEvent,
        window_id: WindowId,
        event_loop: &WindowEventLoop,
    ) -> Result<()> {
        let RenderState::Active { window, surface } = &mut self.render_state else {
            return Ok(());
        };

        if window.id() != window_id {
            return Ok(());
        }

        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                self.cx.resize_surface(surface, size.width, size.height);
                window.request_redraw();
            }

            WindowEvent::RedrawRequested => {
                let size = window.inner_size();
                if (self.renderer)(&mut self.scene, size, &mut self.renderer_state) {
                    let width = surface.config.width;
                    let height = surface.config.height;

                    let surface_texture = surface.surface.get_current_texture()?;

                    let device = &self.cx.devices[surface.dev_id];

                    let renderer = self.renderers[surface.dev_id]
                        .as_mut()
                        .ok_or(anyhow!("Failed to get renderer"))?;

                    renderer
                        .render_to_surface(
                            &device.device,
                            &device.queue,
                            &self.scene,
                            &surface_texture,
                            &RenderParams {
                                base_color: Color::BLACK,
                                width,
                                height,
                                antialiasing_method: AaConfig::Msaa16,
                            },
                        )
                        .map_err(|e| anyhow!(e.to_string()))?;

                    surface_texture.present();
                }
            }

            _ => {}
        }

        Ok(())
    }
}

fn create_window(event_loop: &EventLoop) -> Result<Arc<Window>> {
    Ok(Arc::new(
        WindowBuilder::new()
            .with_inner_size(LogicalSize::new(1920, 1080))
            .with_title("Gosub Browser")
            .build(event_loop)?,
    ))
}
