use std::cell::LazyCell;
use std::ops::Deref;
use std::sync::mpsc::Sender;
use std::sync::Arc;

use crate::application::{CustomEventInternal, WindowOptions};
use crate::tabs::Tabs;
use crate::WinitEventLoopHandle;
use anyhow::anyhow;
use gosub_instance::{EngineInstance, InstanceHandle};
use gosub_interface::chrome::ChromeHandle;
use gosub_interface::config::ModuleConfiguration;
use gosub_interface::eventloop::EventLoopHandle;
use gosub_interface::instance::{Handles, InstanceId};
use gosub_interface::layout::LayoutTree;
use gosub_interface::render_backend::{ImageBuffer, NodeDesc, RenderBackend};
use gosub_shared::geo::SizeU32;
use gosub_shared::types::Result;
use image::imageops::FilterType;
use log::{error, warn};
use url::Url;
use winit::dpi::LogicalSize;
use winit::event::Modifiers;
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::window::{Icon, Window as WinitWindow, WindowId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowState<'a, B: RenderBackend> {
    Active { surface: B::ActiveWindowData<'a> },
    Suspended,
}

thread_local! {
static ICON: LazyCell<Icon> = LazyCell::new(|| {
        let bytes = include_bytes!("../../resources/gosub-logo.png");

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

pub struct Window<'a, C: ModuleConfiguration> {
    pub(crate) state: WindowState<'a, C::RenderBackend>,
    pub(crate) window: Arc<WinitWindow>,
    pub(crate) renderer_data: <C::RenderBackend as RenderBackend>::WindowData<'a>,
    pub(crate) tabs: Tabs,
    pub(crate) mods: Modifiers,
    pub(crate) handles: Handles<C>,
}

impl<'a, C: ModuleConfiguration<ChromeHandle = WinitEventLoopHandle<C>>> Window<'a, C> {
    pub fn new(
        event_loop: &ActiveEventLoop,
        backend: &mut C::RenderBackend,
        opts: WindowOptions,
        mut handles: Handles<C>,
    ) -> Result<Self> {
        let window = create_window(event_loop)?;

        handles.chrome.window = window.id();

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
                        doc.body().and_then(|body| body.append_child(&canvas).ok())
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
            tabs: Tabs::default(),
            mods: Modifiers::default(),
            handles,
        })
    }

    pub fn resumed(&mut self, backend: &mut C::RenderBackend) -> Result<()> {
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

    pub fn suspended(&mut self, _el: &ActiveEventLoop, backend: &mut C::RenderBackend) {
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

    pub fn draw_scene(
        &mut self,
        scene: <C::RenderBackend as RenderBackend>::Scene,
        instance: InstanceId,
        backend: &mut C::RenderBackend,
    ) -> Result<()> {
        let WindowState::Active { surface: data } = &mut self.state else {
            return Ok(());
        };

        if !self.tabs.is_active(instance) {
            return Ok(());
        }

        backend.reset(&mut self.renderer_data);
        backend.apply_scene(&mut self.renderer_data, &scene, None);

        backend.render(&mut self.renderer_data, data)
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
