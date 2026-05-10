use std::fmt::Debug;

use anyhow::anyhow;
use gosub_fontmanager::FontManager;
use log::info;
use vello::kurbo::Point as VelloPoint;
use vello::peniko::Color as VelloColor;
use vello::{AaConfig, RenderParams, Scene as VelloScene};

pub use border::*;
pub use brush::*;
pub use color::*;
use gosub_interface::font::HasFontManager;
use gosub_interface::render_backend::{RenderBackend, RenderRect, RenderText, Scene as TScene, WindowHandle};
use gosub_shared::geo::{Point, SizeU32};
use gosub_shared::types::Result;
pub use gradient::*;
pub use image::*;
pub use rect::*;
pub use scene::*;
pub use text::*;
pub use transform::*;

use crate::render::window::{ActiveWindowData, WindowData};
use crate::render::Renderer;

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
pub mod engine_backend;
#[cfg(feature = "vello_svg")]
mod vello_svg;

pub struct VelloBackend {
    #[cfg(target_arch = "wasm32")]
    renderer: Renderer,
}

impl Debug for VelloBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VelloRenderer").finish()
    }
}

impl HasFontManager for VelloBackend {
    type FontManager = FontManager;
}

impl RenderBackend for VelloBackend {
    type Rect = Rect;
    type Border = Border;
    type BorderSide = BorderSide;
    type BorderRadius = BorderRadius;
    type Transform = Transform;
    type Gradient = Gradient;
    type Color = Color;
    type Image = Image;
    type Brush = Brush;
    type Scene = Scene;
    type Text = Text;
    #[cfg(feature = "resvg")]
    type SVGRenderer = gosub_svg::resvg::Resvg;
    #[cfg(all(feature = "vello_svg", not(feature = "resvg")))]
    type SVGRenderer = vello_svg::VelloSVG;

    type ActiveWindowData<'a> = ActiveWindowData<'a>;
    type WindowData<'a> = WindowData;

    type FontManager = FontManager;

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
        let surface =
            data.adapter
                .create_surface(handle, size.width, size.height, vello::wgpu::PresentMode::AutoVsync)?;

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
        let renderer = futures::executor::block_on(Renderer::new())?;

        let adapter = renderer.instance_adapter;

        let renderer = adapter.create_renderer(None)?;
        let blitter = None;

        info!("Created renderer");

        Ok(WindowData {
            adapter,
            renderer,
            scene: VelloScene::new().into(),
            blitter,
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

        let surface_texture = match active_data.surface.surface.get_current_texture() {
            vello::wgpu::CurrentSurfaceTexture::Success(t) | vello::wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            other => return Err(anyhow!("Failed to acquire surface texture: {:?}", other)),
        };
        let surface_view = surface_texture
            .texture
            .create_view(&vello::wgpu::TextureViewDescriptor::default());

        if window_data.blitter.is_none() {
            window_data.blitter = Some(vello::wgpu::util::TextureBlitter::new(
                &window_data.adapter.device,
                active_data.surface.config.format,
            ));
        }

        window_data
            .renderer
            .render_to_texture(
                &window_data.adapter.device,
                &window_data.adapter.queue,
                &window_data.scene.0,
                &active_data.surface.target_view,
                &RenderParams {
                    base_color: VelloColor::WHITE,
                    width,
                    height,
                    antialiasing_method: AaConfig::Msaa16,
                },
            )
            .map_err(|e| anyhow!(e.to_string()))?;

        let mut encoder = window_data
            .adapter
            .device
            .create_command_encoder(&vello::wgpu::CommandEncoderDescriptor {
                label: Some("vello-blit"),
            });

        window_data
            .blitter
            .as_ref()
            .ok_or_else(|| anyhow!("texture blitter not initialized"))?
            .copy(
                &window_data.adapter.device,
                &mut encoder,
                &active_data.surface.target_view,
                &surface_view,
            );

        window_data.adapter.queue.submit([encoder.finish()]);

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
    #[must_use]
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
        VelloPoint::new(f64::from(self.x), f64::from(self.y))
    }
}
