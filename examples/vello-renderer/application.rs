use std::collections::HashMap;
use std::fmt::{Debug, Formatter};
use std::sync::mpsc;

use crate::window::Window;
use crate::WinitEventLoopHandle;
use anyhow::anyhow;
use gosub_instance::{DebugEvent, InstanceMessage};
use gosub_interface::chrome::ChromeHandle;
use gosub_interface::config::{HasRenderBackend, ModuleConfiguration};
use gosub_interface::instance::{Handles, InstanceId};
use gosub_interface::render_backend::{NodeDesc, RenderBackend, SizeU32};
use gosub_interface::request::RequestServerHandle;
use gosub_shared::types::Result;
use log::{error, info};
use url::Url;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::window::WindowId;

#[derive(Debug, Default)]
pub struct WindowOptions {
    #[cfg(target_arch = "wasm32")]
    pub id: String,
    #[cfg(target_arch = "wasm32")]
    pub parent_id: String,
}

impl WindowOptions {
    #[cfg(target_arch = "wasm32")]
    pub fn with_id(id: String) -> Self {
        Self {
            id,
            parent_id: String::new(),
        }
    }
}

#[allow(clippy::type_complexity)]
pub struct Application<'a, C: ModuleConfiguration> {
    open_windows: Vec<(Vec<Url>, WindowOptions)>, // Vec of Windows, each with a Vec of URLs, representing tabs
    windows: HashMap<WindowId, Window<'a, C>>,
    backend: C::RenderBackend,
    layouter: C::Layouter,
    active_state: Option<ActiveState<C>>,
    event_loop: Option<EventLoop<CustomEventInternal<C>>>,
}

pub struct ActiveState<C: ModuleConfiguration> {
    proxy: EventLoopProxy<CustomEventInternal<C>>,
    handles: Handles<C>,
}

impl<C: ModuleConfiguration> Application<'_, C> {
    fn active_state(&self) -> &ActiveState<C> {
        self.active_state
            .as_ref()
            .expect("No active state; event loop not running!")
    }
}

impl<C: ModuleConfiguration<ChromeHandle = WinitEventLoopHandle<C>>> ApplicationHandler<CustomEventInternal<C>>
    for Application<'_, C>
{
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {
        info!("Resumed");
        for window in self.windows.values_mut() {
            if let Err(e) = window.resumed(&mut self.backend) {
                error!("Error resuming window: {e:?}");
            }
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: CustomEventInternal<C>) {
        match event {
            CustomEventInternal::OpenWindow(url, id) => {
                info!("Opening window with URL: {url}");

                let handles = self.active_state().handles.clone();

                let mut window = match Window::new(event_loop, &mut self.backend, id, handles) {
                    Ok(window) => window,
                    Err(e) => {
                        error!("Error opening window: {e:?}");

                        if self.windows.is_empty() {
                            info!("No more windows; exiting event loop");
                            event_loop.exit();
                        }
                        return;
                    }
                };

                if let Err(e) = window.resumed(&mut self.backend) {
                    error!("Error resuming window: {e:?}");
                    if self.windows.is_empty() {
                        info!("No more windows; exiting event loop");
                        event_loop.exit();
                    }
                    return;
                }
                let id = window.id();

                self.windows.insert(id, window);

                let proxy = self.active_state().proxy.clone();

                info!("Sending OpenTab event");

                let _ = proxy.send_event(CustomEventInternal::OpenTab(url, id));
            }
            CustomEventInternal::OpenTab(url, id) => {
                info!("Opening tab with URL: {url}");

                let mut handles = self.active_state().handles.clone();
                let Some(window) = self.windows.get_mut(&id) else {
                    error!("No window with ID: {id:?}");
                    return;
                };

                handles.chrome.window = window.id();

                if let Err(e) = window.tabs.open(url, self.layouter.clone(), handles) {
                    error!("Error opening tab: {e:?}");
                    return;
                }

                window.request_redraw();
            }
            CustomEventInternal::CloseWindow(id) => {
                self.windows.remove(&id);
                if self.windows.is_empty() {
                    info!("No more windows; exiting event loop");
                    event_loop.exit();
                }
            }
            CustomEventInternal::OpenInitial => {
                info!("Opening initial windows");

                let handles = self.active_state().handles.clone();
                let proxy = self.active_state().proxy.clone();

                for (urls, opts) in self.open_windows.drain(..) {
                    let mut window = match Window::new(event_loop, &mut self.backend, opts, handles.clone()) {
                        Ok(window) => window,
                        Err(e) => {
                            error!("Error opening window: {e:?}");
                            if self.windows.is_empty() {
                                info!("No more windows; exiting event loop");
                                event_loop.exit();
                            }
                            return;
                        }
                    };

                    if let Err(e) = window.resumed(&mut self.backend) {
                        error!("Error resuming window: {e:?}");
                        if self.windows.is_empty() {
                            info!("No more windows; exiting event loop");
                            event_loop.exit();
                        }
                        return;
                    }

                    let id = window.id();

                    self.windows.insert(id, window);

                    info!("Sending OpenTab event");

                    for url in urls {
                        let _ = proxy.send_event(CustomEventInternal::OpenTab(url, id));
                    }
                }
            }
            CustomEventInternal::Debug(event) => {
                if let Some(window) = self.windows.values_mut().next() {
                    let Some(tab) = window.tabs.get_current_tab() else {
                        return;
                    };

                    let _ = tab.tx.blocking_send(InstanceMessage::Debug(event));
                    window.request_redraw();
                }
            }
            CustomEventInternal::DrawScene(scene, _, id, window) => {
                let Some(window) = self.windows.get_mut(&window) else {
                    return;
                };

                let _ = window.draw_scene(scene, id, &mut self.backend);
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
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

impl<'a, C: ModuleConfiguration<ChromeHandle = WinitEventLoopHandle<C>>> Application<'a, C> {
    pub fn new(backend: C::RenderBackend, layouter: C::Layouter) -> Self {
        Self {
            windows: HashMap::new(),
            backend,
            layouter,
            active_state: None,
            event_loop: None,
            open_windows: Vec::new(),
        }
    }

    pub fn initial_tab(&mut self, url: Url, opts: WindowOptions) {
        self.open_windows.push((vec![url], opts));
    }

    pub fn initial(&mut self, mut windows: Vec<(Vec<Url>, WindowOptions)>) {
        self.open_windows.append(&mut windows);
    }

    pub fn add_window(&mut self, window: Window<'a, C>) {
        self.windows.insert(window.window.id(), window);
    }

    pub fn open_window(&mut self, url: Url, opts: WindowOptions) {
        if let Some(state) = self.active_state.as_ref() {
            let _ = state.proxy.send_event(CustomEventInternal::OpenWindow(url, opts));
        }
    }

    pub fn initialize(&mut self) -> Result<()> {
        let event_loop = EventLoop::with_user_event().build()?;

        let proxy = event_loop.create_proxy();

        let handles = Handles {
            chrome: WinitEventLoopHandle {
                proxy: proxy.clone(),
                window: WindowId::from(0),
            },
            request: RequestServerHandle,
        };

        self.active_state = Some(ActiveState { handles, proxy });
        self.event_loop = Some(event_loop);

        Ok(())
    }

    pub fn run(&mut self) -> Result<()> {
        if self.event_loop.is_none() {
            self.initialize()?;
        }

        let event_loop = self.event_loop.take().ok_or(anyhow!("No event loop; unreachable!"))?;

        let proxy = self.proxy()?;

        info!("Sending OpenInitial event");
        proxy
            .send_event(CustomEventInternal::OpenInitial)
            .map_err(|e| anyhow!(e.to_string()))?;

        event_loop.run_app(self)?;

        Ok(())
    }

    #[allow(clippy::type_complexity)]
    pub fn proxy(&mut self) -> Result<EventLoopProxy<CustomEventInternal<C>>> {
        if self.active_state.is_none() {
            self.initialize()?;
        }

        self.active_state
            .as_ref()
            .map(|s| s.proxy.clone())
            .ok_or(anyhow!("No proxy; unreachable!"))
    }

    pub fn close_window(&mut self, id: WindowId) {
        if let Some(state) = &self.active_state {
            let _ = state.proxy.send_event(CustomEventInternal::CloseWindow(id));
        }
    }
}

#[derive(Debug)]
pub enum CustomEvent {
    OpenWindow(Url, WindowOptions),
    OpenTab(Url, WindowId),
    CloseWindow(WindowId),
    OpenInitial,
    Select(u64),
    SendNodes(mpsc::Sender<NodeDesc>),
    Unselect,
}
pub enum CustomEventInternal<C: HasRenderBackend> {
    OpenWindow(Url, WindowOptions),
    OpenTab(Url, WindowId),
    CloseWindow(WindowId),
    OpenInitial,
    Debug(DebugEvent),
    DrawScene(
        <C::RenderBackend as RenderBackend>::Scene,
        SizeU32,
        InstanceId,
        WindowId,
    ),
}

impl<C: HasRenderBackend> Debug for CustomEventInternal<C> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OpenWindow(..) => f.write_str("OpenWindow"),
            Self::OpenTab(..) => f.write_str("OpenTab"),
            Self::CloseWindow(_) => f.write_str("CloseWindow"),
            Self::OpenInitial => f.write_str("OpenInitial"),
            Self::Debug(..) => f.write_str("Debug"),
            Self::DrawScene(..) => f.write_str("DrawScene"),
        }
    }
}
