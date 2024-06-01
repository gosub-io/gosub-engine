use anyhow::anyhow;
use std::collections::HashMap;

use url::Url;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::window::WindowId;

use gosub_render_backend::RenderBackend;
use gosub_renderer::draw::SceneDrawer;
use gosub_shared::types::Result;

use crate::window::Window;

pub struct Application<'a, D: SceneDrawer<B>, B: RenderBackend> {
    open_windows: Vec<Vec<Url>>, // Vec of Windows, each with a Vec of URLs, representing tabs
    windows: HashMap<WindowId, Window<'a, D, B>>,
    backend: B,
    proxy: Option<EventLoopProxy<CustomEvent>>,
}

impl<'a, D: SceneDrawer<B>, B: RenderBackend> ApplicationHandler<CustomEvent>
    for Application<'a, D, B>
{
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {
        for window in self.windows.values_mut() {
            if let Err(e) = window.resumed(&mut self.backend) {
                eprintln!("Error resuming window: {e:?}");
            }
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: CustomEvent) {
        match event {
            CustomEvent::OpenWindow(url) => {
                let mut window = match Window::new(event_loop, &mut self.backend, url) {
                    Ok(window) => window,
                    Err(e) => {
                        eprintln!("Error opening window: {e:?}");
                        return;
                    }
                };

                if let Err(e) = window.resumed(&mut self.backend) {
                    eprintln!("Error resuming window: {e:?}");
                    return;
                }
                self.windows.insert(window.id(), window);
            }
            CustomEvent::CloseWindow(id) => {
                self.windows.remove(&id);
            }
            CustomEvent::OpenInitial => {
                for urls in self.open_windows.drain(..) {
                    let mut window =
                        match Window::new(event_loop, &mut self.backend, urls[0].clone()) {
                            Ok(window) => window,
                            Err(e) => {
                                eprintln!("Error opening window: {e:?}");
                                return;
                            }
                        };

                    if let Err(e) = window.resumed(&mut self.backend) {
                        eprintln!("Error resuming window: {e:?}");
                        return;
                    }

                    self.windows.insert(window.id(), window);
                }
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if let Some(window) = self.windows.get_mut(&window_id) {
            if let Err(e) = window.event(event_loop, &mut self.backend, event) {
                eprintln!("Error handling window event: {e:?}");
            };
        }
    }

    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        for window in self.windows.values_mut() {
            window.suspended(event_loop, &mut self.backend);
        }
    }
}

impl<'a, D: SceneDrawer<B>, B: RenderBackend> Application<'a, D, B> {
    pub fn new(backend: B) -> Self {
        Self {
            windows: HashMap::new(),
            backend,
            proxy: None,
            open_windows: Vec::new(),
        }
    }

    pub fn initial_tab(&mut self, url: Url) {
        self.open_windows.push(vec![url]);
    }

    pub fn initial(&mut self, mut windows: Vec<Vec<Url>>) {
        self.open_windows.append(&mut windows);
    }

    pub fn add_window(&mut self, window: Window<'a, D, B>) {
        self.windows.insert(window.window.id(), window);
    }

    pub fn open_window(&mut self, url: Url) {
        if let Some(proxy) = &self.proxy {
            let _ = proxy.send_event(CustomEvent::OpenWindow(url));
        }
    }

    pub fn start(&mut self) -> Result<()> {
        let event_loop = EventLoop::with_user_event().build()?;

        let proxy = event_loop.create_proxy();

        proxy
            .send_event(CustomEvent::OpenInitial)
            .map_err(|e| anyhow!(e.to_string()))?;

        self.proxy = Some(proxy);

        event_loop.run_app(self)?;

        Ok(())
    }
}

#[derive(Debug)]
enum CustomEvent {
    OpenWindow(Url),
    CloseWindow(WindowId),
    OpenInitial,
}
