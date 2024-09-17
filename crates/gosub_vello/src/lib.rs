use std::fmt::Debug;

use anyhow::anyhow;
use log::info;
use vello::{AaConfig, DebugLayers, RenderParams, Scene as VelloScene};
use vello::kurbo::Point as VelloPoint;
use vello::peniko::Color as VelloColor;

pub use border::*;
pub use brush::*;
pub use color::*;
use gosub_render_backend::{RenderBackend, RenderRect, RenderText, Scene as TScene, WindowHandle};
use gosub_render_backend::geo::{Point, SizeU32};
use gosub_shared::types::Result;
pub use gradient::*;
pub use image::*;
pub use rect::*;
pub use scene::*;
pub use text::*;
pub use transform::*;

use crate::render::{Renderer, RendererOptions};
use crate::render::window::{ActiveWindowData, WindowData};

mod border;
mod brush;
mod color;
mod gradient;
mod image;
mod rect;
mod render;
mod scene;
mod text;
mod transform;

mod debug;
#[cfg(feature = "vello_svg")]
mod vello_svg;

pub struct VelloBackend {
    #[cfg(target_arch = "wasm32")] renderer: Renderer,
}

impl Debug for VelloBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VelloRenderer").finish()
    }
}

impl RenderBackend for VelloBackend {
    type Rect = Rect;
    type Border = Border;
    type BorderSide = BorderSide;
    type BorderRadius = BorderRadius;
    type Transform = Transform;
    type Text = Text;
    type Gradient = Gradient;
    type Color = Color;
    type Image = Image;
    type Brush = Brush;
    type Scene = Scene;
    #[cfg(feature = "resvg")]
    type SVGRenderer = gosub_svg::resvg::Resvg;
    #[cfg(all(feature = "vello_svg", not(feature = "resvg")))]
    type SVGRenderer = vello_svg::VelloSVG;

    type ActiveWindowData<'a> = ActiveWindowData<'a>;

    type WindowData<'a> = WindowData;

    fn draw_rect(&mut self, data: &mut Self::WindowData<'_>, rect: &RenderRect<Self>) {
        data.scene.draw_rect(rect);
    }

    fn draw_text(&mut self, data: &mut Self::WindowData<'_>, text: &RenderText<Self>) {
        data.scene.draw_text(text);
    }

    fn apply_scene(
        &mut self,
        data: &mut Self::WindowData<'_>,
        scene: &Self::Scene,
        transform: Option<Self::Transform>,
    ) {
        data.scene.apply_scene(scene, transform);
    }

    fn reset(&mut self, data: &mut Self::WindowData<'_>) {
        data.scene.reset();
    }

    fn activate_window<'a>(
        &mut self,
        handle: impl WindowHandle + 'a,
        data: &mut Self::WindowData<'_>,
        size: SizeU32,
    ) -> Result<Self::ActiveWindowData<'a>> {
        let surface = data
            .adapter
            .create_surface(handle, size.width, size.height, wgpu::PresentMode::AutoVsync)?;

        let renderer = data.adapter.create_renderer(Some(surface.config.format))?;

        data.renderer = renderer;

        Ok(ActiveWindowData { surface })
    }

    fn suspend_window(
        &mut self,
        _handle: impl WindowHandle,
        _data: &mut Self::ActiveWindowData<'_>,
        _window_data: &mut Self::WindowData<'_>,
    ) -> Result<()> {
        Ok(())
    }

    fn create_window_data<'a>(&mut self, _handle: impl WindowHandle) -> Result<Self::WindowData<'a>> {
        info!("Creating window data");

        #[cfg(target_arch = "wasm32")]
        let renderer = self.renderer.clone();

        #[cfg(not(target_arch = "wasm32"))]
        let renderer = futures::executor::block_on(Renderer::new(RendererOptions::default()))?;

        let adapter = renderer.instance_adapter;

        let renderer = adapter.create_renderer(None)?;

        info!("Created renderer");

        Ok(WindowData {
            adapter,
            renderer,
            scene: VelloScene::new().into(),
        })
    }

    fn resize_window<'a>(
        &mut self,
        window_data: &mut Self::WindowData<'a>,
        active_window_data: &mut Self::ActiveWindowData<'a>,
        size: SizeU32,
    ) -> Result<()> {
        window_data
            .adapter
            .resize_surface(&mut active_window_data.surface, size.width, size.height);

        Ok(())
    }

    fn render<'a>(
        &mut self,
        window_data: &mut Self::WindowData<'a>,
        active_data: &mut Self::ActiveWindowData<'a>,
    ) -> Result<()> {
        let height = active_data.surface.config.height;
        let width = active_data.surface.config.width;

        let surface_texture = active_data.surface.surface.get_current_texture()?;

        window_data
            .renderer
            .render_to_surface(
                &window_data.adapter.device,
                &window_data.adapter.queue,
                &window_data.scene.0,
                &surface_texture,
                &RenderParams {
                    base_color: VelloColor::WHITE,
                    width,
                    height,
                    antialiasing_method: AaConfig::Msaa16,
                    debug: DebugLayers::none(),
                },
            )
            .map_err(|e| anyhow!(e.to_string()))?;

        surface_texture.present();

        Ok(())
    }
}

impl VelloBackend {
    #[cfg(target_arch = "wasm32")]
    pub async fn new() -> Result<Self> {
        let renderer = Renderer::new(RendererOptions::default()).await?;

        Ok(Self { renderer })
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn new() -> Self {
        Self {}
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for VelloBackend {
    fn default() -> Self {
        Self::new()
    }
}

trait Convert<T> {
    fn convert(self) -> T;
}

impl Convert<VelloPoint> for Point {
    fn convert(self) -> VelloPoint {
        VelloPoint::new(self.x as f64, self.y as f64)
    }
}
