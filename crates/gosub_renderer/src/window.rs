use std::sync::Arc;

use anyhow::anyhow;
use vello::peniko::Color;
use vello::{AaConfig, RenderParams, Renderer, Scene};
use winit::dpi::LogicalSize;
use winit::event::{Event, WindowEvent};
use winit::event_loop;
use winit::event_loop::EventLoopWindowTarget;
use winit::window::{Window as WinitWindow, WindowBuilder, WindowId};

use gosub_shared::types::Result;

use crate::draw::SceneDrawer;
use crate::renderer::{InstanceAdapter, SurfaceWrapper, RENDERER_CONF};

pub enum WindowState<'a> {
    Active {
        surface: SurfaceWrapper<'a>,
        window: Arc<WinitWindow>,
    },
    Suspended(Arc<WinitWindow>),
}

type CustomEvent = ();
type EventLoop = event_loop::EventLoop<CustomEvent>;
type WindowEventLoop = EventLoopWindowTarget<CustomEvent>;

pub struct Window<'a, D: SceneDrawer> {
    event_loop: Option<EventLoop>,
    state: WindowState<'a>,
    scene: Scene,
    adapter: Arc<InstanceAdapter>,
    renderer: Renderer,
    scene_drawer: D,
}

impl<'a, D: SceneDrawer> Window<'a, D> {
    /// Creates a new window AND opens it
    pub fn new(
        adapter: Arc<InstanceAdapter>,
        scene_drawer: D,
        #[cfg(target_arch = "wasm32")] canvas_parent_id: Option<String>,
    ) -> Result<Self> {
        let event_loop = EventLoop::new()?;

        let window = create_window(&event_loop)?;

        #[cfg(target_arch = "wasm32")]
        {
            use winit::platform::web::WindowExtWebSys;
            let canvas = window
                .canvas()
                .ok_or(anyhow!("failed to get window canvas"))?;
            web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| {
                    if let Some(id) = canvas_parent_id {
                        d.get_element_by_id(&id)
                    } else {
                        d.body().map(|b| b.into())
                    }
                })
                .and_then(|b| b.append_child(&canvas).ok());

            let _ = web_sys::HtmlElement::from(canvas).focus();
        }

        let state = WindowState::Suspended(window);

        let renderer = adapter.create_renderer()?;

        Ok(Self {
            event_loop: Some(event_loop),
            state,
            scene: Scene::new(),
            adapter,
            renderer,
            scene_drawer,
        })
    }

    pub fn change_adapter(&mut self, adapter: Arc<InstanceAdapter>) {
        self.adapter = adapter;
    }

    /// Starts the window using up the event loop
    /// Returns Ok(true) if the window was closed
    /// Returns Ok(false) if the window was already opened
    pub fn start(&mut self) -> Result<bool> {
        let error = &mut None;

        let Some(event_loop) = self.event_loop.take() else {
            return Ok(false);
        };

        event_loop.run(|event, event_loop| {
            if let Err(e) = self.handle_event(event, event_loop) {
                *error = Some(e);
                event_loop.exit();
            }
        })?;

        if let Some(e) = error.take() {
            return Err(e);
        }

        Ok(true)
    }

    fn handle_event(
        &mut self,
        event: Event<CustomEvent>,
        event_loop: &WindowEventLoop,
    ) -> Result<()> {
        match event {
            Event::Resumed => {
                let WindowState::Suspended(window) = &self.state else {
                    return Ok(());
                };

                let size = window.inner_size();

                let surface = self.adapter.create_surface(
                    Arc::clone(window),
                    size.width,
                    size.height,
                    wgpu::PresentMode::AutoVsync,
                )?;

                let mut conf = RENDERER_CONF;

                conf.surface_format = Some(surface.config.format);

                self.renderer = Renderer::new(&self.adapter.device, conf)
                    .map_err(|e| anyhow!(e.to_string()))?;

                self.state = WindowState::Active {
                    surface,
                    window: Arc::clone(window),
                };

                event_loop.set_control_flow(event_loop::ControlFlow::Poll);
            }
            Event::Suspended => {
                if let WindowState::Active { window, .. } = &self.state {
                    self.state = WindowState::Suspended(window.clone());
                };

                event_loop.set_control_flow(event_loop::ControlFlow::Wait);
            }

            Event::AboutToWait => {
                if let WindowState::Active { window, .. } = &self.state {
                    window.request_redraw();
                }
            }
            Event::WindowEvent { event, window_id } => {
                self.handle_window_event(event, window_id, event_loop)?;
            }
            _ => {}
        }

        Ok(())
    }

    fn handle_window_event(
        &mut self,
        event: WindowEvent,
        window_id: WindowId,
        event_loop: &WindowEventLoop,
    ) -> Result<()> {
        let WindowState::Active { window, surface } = &mut self.state else {
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
                self.adapter
                    .resize_surface(surface, size.width, size.height);
                window.request_redraw();
            }

            WindowEvent::RedrawRequested => {
                let size = window.inner_size();
                if self.scene_drawer.draw(&mut self.scene, size) {
                    let width = surface.config.width;
                    let height = surface.config.height;

                    let surface_texture = surface.surface.get_current_texture()?;

                    self.renderer
                        .render_to_surface(
                            &self.adapter.device,
                            &self.adapter.queue,
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

fn create_window(event_loop: &EventLoop) -> Result<Arc<WinitWindow>> {
    Ok(Arc::new(
        WindowBuilder::new()
            .with_inner_size(LogicalSize::new(1920, 1080))
            .with_title("Gosub Browser")
            .build(event_loop)?,
    ))
}
