use std::fmt::Debug;
use std::sync::Arc;
pub use border::*;
pub use brush::*;
pub use color::*;
use gosub_shared::render_backend::geo::{SizeU32};
use gosub_shared::render_backend::{RenderBackend, RenderRect, RenderText, WindowHandle};
use gosub_shared::types::Result;
pub use gradient::*;
pub use image::*;
pub use rect::*;
pub use scene::*;
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
mod scene;
mod text;
mod transform;
mod debug;

pub struct CairoBackend {}

impl Debug for CairoBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CairoRenderer").finish()
    }
}

impl RenderBackend for CairoBackend {
    type Rect = Rect;
    type Border = Border;
    type BorderSide = BorderSide;
    type BorderRadius = BorderRadius;
    type Transform = Transform;
    type Text = Text;
    type Gradient = CairoGradient;
    type Color = Color;
    type Image = Image;
    type Brush = Brush;
    type Scene = Scene;
    type SVGRenderer = gosub_svg::resvg::Resvg;

    type ActiveWindowData<'a> = ActiveWindowData<'a>;
    type WindowData<'a> = WindowData<'a>;

    fn draw_rect(&mut self, _data: &mut Self::WindowData<'_>, _rect: &RenderRect<Self>) {
        println!("render_backend::draw_rect");
    }

    fn draw_text(&mut self, _data: &mut Self::WindowData<'_>, _text: &RenderText<Self>) {
        // data.draw_text(text);
        println!("render_backend::draw_text:");
    }

    fn apply_scene(
        &mut self,
        _data: &mut Self::WindowData<'_>,
        _scene: &Self::Scene,
        _transform: Option<Self::Transform>,
    ) {
        println!("apply scene");
        // data.apply_scene(scene, transform);
    }

    fn reset(&mut self, _data: &mut Self::WindowData<'_>) {
        println!("reset");
        // data.reset();
    }

    fn activate_window<'a>(
        &mut self,
        _handle: impl WindowHandle + 'a,
        data: &mut Self::WindowData<'_>,
        _size: SizeU32,
    ) -> Result<Self::ActiveWindowData<'a>> {
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 800, 600).unwrap();

        Ok(ActiveWindowData {
            surface: surface.clone(),
            crc: data.crc.clone(),
            _phantom: std::marker::PhantomData,
        })
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

        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 800, 600).unwrap();
        let ctx = cairo::Context::new(&surface).unwrap();

        let crc = CairoRenderContext::new(ctx);

        Ok(WindowData {
            crc: Arc::new(crc),
            _phantom: std::marker::PhantomData,
        })
    }

    fn resize_window(&mut self, _window_data: &mut Self::WindowData<'_>, _active_window_data: &mut Self::ActiveWindowData<'_>, _size: SizeU32) -> Result<()> {
        Ok(())
    }

    fn render(&mut self, _window_data: &mut Self::WindowData<'_>, _active_data: &mut Self::ActiveWindowData<'_>) -> Result<()> {
        Ok(())
    }


    // fn resize_window<'a>(
    //     &mut self,
    //     _window_data: &mut Self::WindowData<'a>,
    //     _active_window_data: &mut Self::ActiveWindowData<'a>,
    //     _size: SizeU32,
    // ) -> Result<()> {
    //     Ok(())
    // }
    //
    // fn render<'a>(
    //     &mut self,
    //     _window_data: &mut Self::WindowData<'a>,
    //     _active_data: &mut Self::ActiveWindowData<'a>,
    // ) -> Result<()> {
    //     Ok(())
    // }
}

impl CairoBackend {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for CairoBackend {
    fn default() -> Self {
        Self::new()
    }
}