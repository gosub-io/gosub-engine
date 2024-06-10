use std::fmt::Debug;

use anyhow::anyhow;
use vello::kurbo::{Point as VelloPoint, RoundedRect};
use vello::peniko::{Color as VelloColor, Fill};
use vello::{AaConfig, RenderParams, Scene};

use crate::render::{Renderer, RendererOptions};
pub use border::*;
pub use brush::*;
pub use color::*;
use gosub_render_backend::{Point, RenderBackend, RenderRect, RenderText, SizeU32, WindowHandle};
use gosub_shared::types::Result;
pub use gradient::*;
pub use image::*;
pub use rect::*;
pub use text::*;
pub use transform::*;

use crate::render::window::{ActiveWindowData, WindowData};

mod border;
mod brush;
mod color;
mod gradient;
mod image;
mod rect;
mod render;
mod text;
mod transform;

pub struct VelloBackend;

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
    type PreRenderText = PreRenderText;
    type Text = Text;
    type Gradient = Gradient;
    type Color = Color;
    type Image = Image;
    type Brush = Brush;
    type ActiveWindowData<'a> = ActiveWindowData<'a>;
    type WindowData<'a> = WindowData;

    fn draw_rect(&mut self, data: &mut Self::WindowData<'_>, rect: &RenderRect<Self>) {
        let affine = rect.transform.as_ref().map(|t| t.0).unwrap_or_default();

        let brush = &rect.brush.0;
        let brush_transform = rect.brush_transform.as_ref().map(|t| t.0);

        if let Some(radius) = &rect.radius {
            let shape = RoundedRect::from_rect(rect.rect.0, radius.clone());
            data.scene
                .fill(Fill::NonZero, affine, brush, brush_transform, &shape)
        } else {
            data.scene
                .fill(Fill::NonZero, affine, brush, brush_transform, &rect.rect.0)
        }

        if let Some(border) = &rect.border {
            let opts = BorderRenderOptions {
                border,
                rect: &rect.rect,
                transform: rect.transform.as_ref(),
                radius: rect.radius.as_ref(),
            };

            Border::draw(&mut data.scene, opts);
        }
    }

    fn draw_text(&mut self, data: &mut Self::WindowData<'_>, text: &RenderText<Self>) {
        Text::show(&mut data.scene, text)
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
        let surface = data.adapter.create_surface(
            handle,
            size.width,
            size.height,
            wgpu::PresentMode::AutoVsync,
        )?;

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

    fn create_window_data<'a>(
        &mut self,
        _handle: impl WindowHandle,
    ) -> Result<Self::WindowData<'a>> {
        let renderer = futures::executor::block_on(Renderer::new(RendererOptions::default()))?;

        let adapter = renderer.instance_adapter;

        let renderer = adapter.create_renderer(None)?;

        Ok(WindowData {
            adapter,
            renderer,
            scene: Scene::new(),
        })
    }

    fn resize_window<'a>(
        &mut self,
        window_data: &mut Self::WindowData<'a>,
        active_window_data: &mut Self::ActiveWindowData<'a>,
        size: SizeU32,
    ) -> Result<()> {
        window_data.adapter.resize_surface(
            &mut active_window_data.surface,
            size.width,
            size.height,
        );

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
                &window_data.scene,
                &surface_texture,
                &RenderParams {
                    base_color: VelloColor::BLACK,
                    width,
                    height,
                    antialiasing_method: AaConfig::Msaa16,
                },
            )
            .map_err(|e| anyhow!(e.to_string()))?;

        surface_texture.present();

        Ok(())
    }
}

impl VelloBackend {
    pub fn new() -> Self {
        Self
    }
}

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
