use std::collections::HashMap;
use std::fmt::{Debug, Formatter};
use std::sync::mpsc;

use anyhow::anyhow;
use log::{error, info};
use url::Url;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::window::WindowId;

use gosub_render_backend::layout::{LayoutTree, Layouter};
use gosub_render_backend::{ImageBuffer, ImgCache, NodeDesc, RenderBackend, SizeU32};
use gosub_renderer::draw::SceneDrawer;
use gosub_shared::traits::css3::CssSystem;
use gosub_shared::traits::document::Document;
use gosub_shared::traits::html5::Html5Parser;
use gosub_shared::traits::render_tree::RenderTree;
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

#[allow(clippy::type_complexity)]
pub struct Application<
    'a,
    D: SceneDrawer<B, L, LT, Doc, C, RT>,
    B: RenderBackend,
    L: Layouter,
    LT: LayoutTree<L>,
    Doc: Document<C>,
    C: CssSystem,
    P: Html5Parser<C, Document = Doc>,
    RT: RenderTree<C>,
> {
    open_windows: Vec<(Vec<Url>, WindowOptions)>, // Vec of Windows, each with a Vec of URLs, representing tabs
    windows: HashMap<WindowId, Window<'a, D, B, L, LT, Doc, C, RT>>,
    backend: B,
    layouter: L,
    #[allow(clippy::type_complexity)]
    proxy: Option<EventLoopProxy<CustomEventInternal<D, B, L, LT, Doc, C, RT>>>,
    #[allow(clippy::type_complexity)]
    event_loop: Option<EventLoop<CustomEventInternal<D, B, L, LT, Doc, C, RT>>>,
    debug: bool,
    _marker: std::marker::PhantomData<&'a P>,
}

impl<
        D: SceneDrawer<B, L, LT, Doc, C, RT>,
        B: RenderBackend,
        L: Layouter,
        LT: LayoutTree<L>,
        Doc: Document<C>,
        C: CssSystem,
        P: Html5Parser<C, Document = Doc>,
        RT: RenderTree<C>,
    > ApplicationHandler<CustomEventInternal<D, B, L, LT, Doc, C, RT>> for Application<'_, D, B, L, LT, Doc, C, P, RT>
{
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {
        info!("Resumed");
        for window in self.windows.values_mut() {
            if let Err(e) = window.resumed(&mut self.backend) {
                error!("Error resuming window: {e:?}");
            }
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: CustomEventInternal<D, B, L, LT, Doc, C, RT>) {
        match event {
            CustomEventInternal::OpenWindow(url, id) => {
                info!("Opening window with URL: {url}");

                let mut window = match Window::new::<P>(
                    event_loop,
                    &mut self.backend,
                    id,
                    self.proxy.clone().expect("unreachable"),
                ) {
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
                if let Some(window) = self.windows.get_mut(&id) {
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
                    let mut window =
                        match Window::new::<P>(event_loop, &mut self.backend, opts, self.proxy.clone().unwrap()) {
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
            CustomEventInternal::Info(id, sender) => {
                if let Some(window) = self.windows.values_mut().next() {
                    window.info(LT::NodeId::from(id), sender);
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

            CustomEventInternal::Redraw(id) => {
                if let Some(window) = self.windows.get_mut(&id) {
                    window.request_redraw();
                }
            }

            CustomEventInternal::AddImg(url, img, size, id) => {
                if let Some(window) = self.windows.get_mut(&id) {
                    if let Some(tab) = window.tabs.get_current_tab() {
                        tab.data.get_img_cache().add(url, img, size);

                        tab.data.make_dirty();

                        tab.data.delete_scene();

                        window.request_redraw();
                    }
                }
            }
            CustomEventInternal::ReloadFrom(rt, id) => {
                if let Some(window) = self.windows.get_mut(&id) {
                    if let Some(tab) = window.tabs.get_current_tab() {
                        tab.reload_from(rt);

                        window.request_redraw();
                    }
                }
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
        if let Some(window) = self.windows.get_mut(&window_id) {
            if let Err(e) = window.event::<P>(event_loop, &mut self.backend, event) {
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
        D: SceneDrawer<B, L, LT, Doc, C, RT>,
        B: RenderBackend,
        L: Layouter,
        LT: LayoutTree<L>,
        Doc: Document<C>,
        C: CssSystem,
        P: Html5Parser<C, Document = Doc>,
        RT: RenderTree<C>,
    > Application<'a, D, B, L, LT, Doc, C, P, RT>
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

    pub fn add_window(&mut self, window: Window<'a, D, B, L, LT, Doc, C, RT>) {
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
    pub fn proxy(&mut self) -> Result<EventLoopProxy<CustomEventInternal<D, B, L, LT, Doc, C, RT>>> {
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
pub enum CustomEventInternal<
    D: SceneDrawer<B, L, LT, Doc, C, RT>,
    B: RenderBackend,
    L: Layouter,
    LT: LayoutTree<L>,
    Doc: Document<C>,
    C: CssSystem,
    RT: RenderTree<C>,
> {
    OpenWindow(Url, WindowOptions),
    OpenTab(Url, WindowId),
    AddTab(Tab<D, B, L, LT, Doc, C, RT>, WindowId),
    CloseWindow(WindowId),
    OpenInitial,
    Select(u64),
    Info(u64, mpsc::Sender<NodeDesc>),
    SendNodes(mpsc::Sender<NodeDesc>),
    Unselect,
    Redraw(WindowId),
    AddImg(String, ImageBuffer<B>, Option<SizeU32>, WindowId),
    ReloadFrom(RT, WindowId),
}

impl<
        D: SceneDrawer<B, L, LT, Doc, C, RT>,
        B: RenderBackend,
        L: Layouter,
        LT: LayoutTree<L>,
        Doc: Document<C>,
        C: CssSystem,
        RT: RenderTree<C>,
    > Debug for CustomEventInternal<D, B, L, LT, Doc, C, RT>
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OpenWindow(..) => f.write_str("OpenWindow"),
            Self::OpenTab(..) => f.write_str("OpenTab"),
            Self::AddTab(..) => f.write_str("AddTab"),
            Self::CloseWindow(_) => f.write_str("CloseWindow"),
            Self::OpenInitial => f.write_str("OpenInitial"),
            Self::Select(_) => f.write_str("Select"),
            Self::SendNodes(_) => f.write_str("SendNodes"),
            Self::Unselect => f.write_str("Unselect"),
            Self::Redraw(_) => f.write_str("Redraw"),
            Self::AddImg(..) => f.write_str("AddImg"),
            Self::ReloadFrom(..) => f.write_str("ReloadFrom"),
            Self::Info(..) => f.write_str("Info"),
        }
    }
}
