use std::cell::LazyCell;
use std::ops::Deref;
use std::sync::Arc;
use std::sync::mpsc::Sender;

use anyhow::anyhow;
use image::imageops::FilterType;
use log::warn;
use url::Url;
use winit::dpi::LogicalSize;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Icon, Window as WinitWindow, WindowId};

use gosub_render_backend::{NodeDesc, RenderBackend};
use gosub_render_backend::geo::SizeU32;
use gosub_render_backend::layout::{Layouter, LayoutTree};
use gosub_renderer::draw::SceneDrawer;
use gosub_shared::traits::css3::CssSystem;
use gosub_shared::traits::document::Document;
use gosub_shared::traits::html5::Html5Parser;
use gosub_shared::types::Result;

use crate::application::WindowOptions;
use crate::tabs::{Tab, TabID, Tabs};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowState<'a, B: RenderBackend> {
    Active { surface: B::ActiveWindowData<'a> },
    Suspended,
}

thread_local! {
static ICON: LazyCell<Icon> = LazyCell::new(|| {
        let bytes = include_bytes!("../../../resources/gosub-logo.png");

        let Ok(img) = image::load_from_memory(bytes) else {
            return Icon::from_rgba(vec![], 0, 0).unwrap();
        };


        let height = img.height() / (img.width() / 256);

        let rgba = img.resize_exact(256, height, FilterType::Nearest).to_rgba8();


        Icon::from_rgba(rgba.to_vec(), rgba.width(), rgba.height()).unwrap_or(
            Icon::from_rgba(vec![], 0, 0).unwrap()
        )

});
}

pub struct Window<
    'a,
    D: SceneDrawer<B, L, LT, Doc, C>,
    B: RenderBackend,
    L: Layouter,
    LT: LayoutTree<L>,
    Doc: Document<C>,
    C: CssSystem,
> {
    pub(crate) state: WindowState<'a, B>,
    pub(crate) window: Arc<WinitWindow>,
    pub(crate) renderer_data: B::WindowData<'a>,
    pub(crate) tabs: Tabs<D, B, L, LT, Doc, C>,
}

impl<
        'a,
        D: SceneDrawer<B, L, LT, Doc, C>,
        B: RenderBackend,
        L: Layouter,
        LT: LayoutTree<L>,
        Doc: Document<C>,
        C: CssSystem,
    > Window<'a, D, B, L, LT, Doc, C>
{
    pub fn new<P: Html5Parser<C, Document = Doc>>(
        event_loop: &ActiveEventLoop,
        backend: &mut B,
        opts: WindowOptions,
    ) -> Result<Self> {
        let window = create_window(event_loop)?;


        #[cfg(target_arch = "wasm32")]
        {
            use winit::platform::web::WindowExtWebSys;
            let canvas = window.canvas().ok_or(anyhow!("Failed to get canvas"))?;
            let size = window.inner_size();
            canvas.set_id(&opts.id);

            canvas.set_height(size.height.max(400));
            canvas.set_width(size.width.max(600));

            
            web_sys::window()
                .and_then(|win| win.document())
                .and_then(|doc| {
                    
                    if !opts.parent_id.is_empty() {
                        doc.get_element_by_id(&opts.parent_id)
                            .and_then(|el| el.append_child(&canvas).ok())
                    } else {
                        doc.body()
                            .and_then(|body| body.append_child(&canvas).ok())
                    }
                })
                .ok_or(anyhow!("Failed to append canvas to body"))?;
        }
        
        #[cfg(not(target_arch = "wasm32"))]
        let _ = opts;

        
        let renderer_data = backend.create_window_data(window.clone())?;

        Ok(Self {
            state: WindowState::Suspended,
            window,
            renderer_data,
            tabs: Tabs::from_url::<P>(default_url, layouter, debug).await?,
        })
    }

    pub async fn open_tab(&mut self, url: Url, layouter: L, debug: bool) -> Result<()> {
        let tab = Tab::from_url(url, layouter, debug).await?;
        self.tabs.add_tab(tab);
        Ok(())
    }

    pub fn add_tab(&mut self, tab: Tab<D, B, L, LT>) {
        let id = self.tabs.add_tab(tab);

        if self.tabs.active == TabID::default() {
            self.tabs.activate_tab(id);
        }


        self.window.request_redraw();
    }

    pub fn resumed(&mut self, backend: &mut B) -> Result<()> {
        if !matches!(self.state, WindowState::Suspended) {
            return Ok(());
        };

        let size = self.window.inner_size();
        let size = SizeU32::new(size.width, size.height);

        let data = backend.activate_window(self.window.clone(), &mut self.renderer_data, size)?;

        self.state = WindowState::Active { surface: data };
        
        // #[cfg(target_arch = "wasm32")]
        // self.request_redraw();

        Ok(())
    }

    pub fn suspended(&mut self, _el: &ActiveEventLoop, backend: &mut B) {
        let WindowState::Active { surface: data } = &mut self.state else {
            return;
        };

        if let Err(e) = backend.suspend_window(self.window.clone(), data, &mut self.renderer_data) {
            warn!("Failed to suspend window: {}", e);
        }

        self.state = WindowState::Suspended;
    }

    pub fn id(&self) -> WindowId {
        self.window.id()
    }

    pub fn request_redraw(&self) {
        self.window.request_redraw();
    }

    pub fn state(&self) -> &'static str {
        match self.state {
            WindowState::Active { .. } => "Active",
            WindowState::Suspended => "Suspended",
        }
    }

    pub fn select_element(&mut self, id: LT::NodeId) {
        self.tabs.select_element(id);
    }

    pub fn send_nodes(&mut self, sender: Sender<NodeDesc>) {
        self.tabs.send_nodes(sender);
    }

    pub fn unselect_element(&mut self) {
        self.tabs.unselect_element();
    }
}

fn create_window(event_loop: &ActiveEventLoop) -> Result<Arc<WinitWindow>> {
    let attributes = WinitWindow::default_attributes()
        .with_title("Gosub Browser")
        .with_window_icon(Some(ICON.with(|icon| icon.deref().clone())))
        .with_inner_size(LogicalSize::new(1920, 1080));

    event_loop
        .create_window(attributes)
        .map_err(|e| anyhow!(e.to_string()))
        .map(Arc::new)
}
