use anyhow::anyhow;
use image::imageops::FilterType;
use log::warn;
use std::cell::LazyCell;
use std::ops::Deref;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use url::Url;
use winit::dpi::LogicalSize;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Icon, Window as WinitWindow, WindowId};

use gosub_render_backend::geo::SizeU32;
use gosub_render_backend::layout::{LayoutTree, Layouter};
use gosub_render_backend::{NodeDesc, RenderBackend};
use gosub_renderer::draw::SceneDrawer;
use gosub_shared::types::Result;

use crate::tabs::Tabs;

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

pub struct Window<'a, D: SceneDrawer<B, L, LT>, B: RenderBackend, L: Layouter, LT: LayoutTree<L>> {
    pub(crate) state: WindowState<'a, B>,
    pub(crate) window: Arc<WinitWindow>,
    pub(crate) renderer_data: B::WindowData<'a>,
    pub(crate) tabs: Tabs<D, B, L, LT>,
}

impl<'a, D: SceneDrawer<B, L, LT>, B: RenderBackend, L: Layouter, LT: LayoutTree<L>>
    Window<'a, D, B, L, LT>
{
    pub fn new(
        event_loop: &ActiveEventLoop,
        backend: &mut B,
        layouter: L,
        default_url: Url,
        debug: bool,
    ) -> Result<Self> {
        let window = create_window(event_loop)?;

        let renderer_data = backend.create_window_data(window.clone())?;

        Ok(Self {
            state: WindowState::Suspended,
            window,
            renderer_data,
            tabs: Tabs::from_url(default_url, layouter, debug)?,
        })
    }

    pub fn resumed(&mut self, backend: &mut B) -> Result<()> {
        if !matches!(self.state, WindowState::Suspended) {
            return Ok(());
        };

        let size = self.window.inner_size();
        let size = SizeU32::new(size.width, size.height);

        let data = backend.activate_window(self.window.clone(), &mut self.renderer_data, size)?;

        self.state = WindowState::Active { surface: data };

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
