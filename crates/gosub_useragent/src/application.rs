use std::collections::HashMap;
use std::sync::mpsc;

use anyhow::anyhow;
use log::{error, info};
use url::Url;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::window::WindowId;

use gosub_render_backend::layout::{LayoutTree, Layouter};
use gosub_render_backend::{NodeDesc, RenderBackend};
use gosub_renderer::draw::SceneDrawer;
use gosub_shared::traits::css3::CssSystem;
use gosub_shared::traits::document::Document;
use gosub_shared::traits::html5::Html5Parser;
use gosub_shared::types::Result;

use crate::tabs::Tab;
use crate::window::Window;

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

pub struct Application<
    'a,
    D: SceneDrawer<B, L, LT, Doc, C>,
    B: RenderBackend,
    L: Layouter,
    LT: LayoutTree<L>,
    Doc: Document<C>,
    C: CssSystem,
    P: Html5Parser<C, Document = Doc>,
> {
    open_windows: Vec<(Vec<Url>, WindowOptions)>, // Vec of Windows, each with a Vec of URLs, representing tabs
    windows: HashMap<WindowId, Window<'a, D, B, L, LT, Doc, C>>,
    backend: B,
    layouter: L,
    #[allow(clippy::type_complexity)]
    proxy: Option<EventLoopProxy<CustomEventInternal<D, B, L, LT, Doc, C>>>,
    #[allow(clippy::type_complexity)]
    event_loop: Option<EventLoop<CustomEventInternal<D, B, L, LT, Doc, C>>>,
    debug: bool,
    _marker: std::marker::PhantomData<&'a P>,
}

impl<
        'a,
        D: SceneDrawer<B, L, LT, Doc, C>,
        B: RenderBackend,
        L: Layouter,
        LT: LayoutTree<L>,
        Doc: Document<C>,
        C: CssSystem,
        P: Html5Parser<C, Document = Doc>,
    > ApplicationHandler<CustomEventInternal<D, B, L, LT, Doc, C>> for Application<'a, D, B, L, LT, Doc, C, P>
{
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {
        info!("Resumed");
        for window in self.windows.values_mut() {
            if let Err(e) = window.resumed(&mut self.backend) {
                error!("Error resuming window: {e:?}");
            }
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: CustomEventInternal<D, B, L, LT, Doc, C>) {
        match event {
            CustomEventInternal::OpenWindow(url, id) => {
                info!("Opening window with URL: {url}");

                let mut window = match Window::new::<P>(event_loop, &mut self.backend, id) {
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

                let Some(proxy) = self.proxy.clone() else {
                    error!("No proxy; unreachable!");
                    return;
                };

                info!("Sending OpenTab event");

                let _ = proxy.send_event(CustomEventInternal::OpenTab(url, id));
            }
            CustomEventInternal::AddTab(tab, id) => {
                info!("Adding tab to window: {id:?}");

                if let Some(window) = self.windows.get_mut(&id) {
                    info!("Found window, adding tab");

                    window.add_tab(tab);
                }
            }
            CustomEventInternal::OpenTab(url, id) => {
                info!("Opening tab with URL: {url}");

                let Some(proxy) = self.proxy.clone() else {
                    return;
                };

                let layouter = self.layouter.clone();
                let debug = self.debug;

                gosub_shared::async_executor::spawn(async move {
                    let tab = match Tab::from_url::<P>(url, layouter, debug).await {
                        Ok(tab) => tab,
                        Err(e) => {
                            error!("Error opening tab: {e:?}");
                            return;
                        }
                    };

                    let _ = proxy.send_event(CustomEventInternal::AddTab(tab, id));
                });
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

                for (urls, opts) in self.open_windows.drain(..) {
                    let mut window = match Window::new::<P>(event_loop, &mut self.backend, opts) {
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

                    let Some(proxy) = self.proxy.clone() else {
                        error!("No proxy; unreachable!");
                        return;
                    };

                    info!("Sending OpenTab event");

                    for url in urls {
                        let _ = proxy.send_event(CustomEventInternal::OpenTab(url, id));
                    }
                }
            }
            CustomEventInternal::Select(id) => {
                if let Some(window) = self.windows.values_mut().next() {
                    window.select_element(LT::NodeId::from(id));
                    window.request_redraw();
                }
            }
            CustomEventInternal::SendNodes(sender) => {
                for window in self.windows.values_mut() {
                    window.send_nodes(sender.clone());
                }
            }

            CustomEventInternal::Unselect => {
                if let Some(window) = self.windows.values_mut().next() {
                    window.unselect_element();
                    window.request_redraw();
                }
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

impl<
        'a,
        D: SceneDrawer<B, L, LT, Doc, C>,
        B: RenderBackend,
        L: Layouter,
        LT: LayoutTree<L>,
        Doc: Document<C>,
        C: CssSystem,
        P: Html5Parser<C, Document = Doc>,
    > Application<'a, D, B, L, LT, Doc, C, P>
{
    pub fn new(backend: B, layouter: L, debug: bool) -> Self {
        Self {
            windows: HashMap::new(),
            backend,
            layouter,
            proxy: None,
            event_loop: None,
            open_windows: Vec::new(),
            debug,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn initial_tab(&mut self, url: Url, opts: WindowOptions) {
        self.open_windows.push((vec![url], opts));
    }

    pub fn initial(&mut self, mut windows: Vec<(Vec<Url>, WindowOptions)>) {
        self.open_windows.append(&mut windows);
    }

    pub fn add_window(&mut self, window: Window<'a, D, B, L, LT, Doc, C>) {
        self.windows.insert(window.window.id(), window);
    }

    pub fn open_window(&mut self, url: Url, opts: WindowOptions) {
        if let Some(proxy) = &self.proxy {
            let _ = proxy.send_event(CustomEventInternal::OpenWindow(url, opts));
        }
    }

    pub fn initialize(&mut self) -> Result<()> {
        let event_loop = EventLoop::with_user_event().build()?;

        self.proxy = Some(event_loop.create_proxy());
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
    pub fn proxy(&mut self) -> Result<EventLoopProxy<CustomEventInternal<D, B, L, LT, Doc, C>>> {
        if self.proxy.is_none() {
            self.initialize()?;
        }

        self.proxy.clone().ok_or(anyhow!("No proxy; unreachable!"))
    }

    pub fn close_window(&mut self, id: WindowId) {
        if let Some(proxy) = &self.proxy {
            let _ = proxy.send_event(CustomEventInternal::CloseWindow(id));
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
#[derive(Debug)]
pub enum CustomEventInternal<
    D: SceneDrawer<B, L, LT, Doc, C>,
    B: RenderBackend,
    L: Layouter,
    LT: LayoutTree<L>,
    Doc: Document<C>,
    C: CssSystem,
> {
    OpenWindow(Url, WindowOptions),
    OpenTab(Url, WindowId),
    AddTab(Tab<D, B, L, LT, Doc, C>, WindowId),
    CloseWindow(WindowId),
    OpenInitial,
    Select(u64),
    SendNodes(mpsc::Sender<NodeDesc>),
    Unselect,
}
