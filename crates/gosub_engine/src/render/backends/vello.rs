use crate::engine::BrowsingContext;
use crate::render::backend::GpuPixelFormat;
use crate::render::backend::{ErasedSurface, ExternalHandle, PresentMode, RenderBackend, RgbaImage, SurfaceSize};
use crate::render::backends::vello::font_cache::FontCache;
use crate::render::backends::vello::font_manager::FontManager;
use crate::render::backends::vello::text_renderer::{TextKey, TextRenderer};
use crate::render::DisplayItem;
use anyhow::{anyhow, Result};
use std::any::Any;
use std::cell::RefCell;
use std::sync::Arc;
use vello::kurbo::Affine;
use vello::peniko::{Color, Fill};
use vello::wgpu;
use vello::{RenderParams, Renderer, RendererOptions, Scene};

mod font_cache;
mod font_manager;
mod text_renderer;

/// This trait abstracts over the wgpu context (device, queue, texture management) so we can connect
/// UI based wgpu contexts (like eframe) to the Vello backend.
pub trait WgpuContextProvider {
    fn device(&self) -> &wgpu::Device;
    fn queue(&self) -> &wgpu::Queue;
    fn create_texture(&self, width: u32, height: u32, format: wgpu::TextureFormat) -> u64;
    fn get_texture(&self, id: u64) -> Option<(wgpu::Texture, wgpu::TextureView)>;
    fn remove_texture(&self, id: u64);
}

/// A render backend that uses Vello for rendering.
pub struct VelloBackend<C: WgpuContextProvider + Send + Sync> {
    /// The wgpu context provider that we can use for device, queue, and texture management.
    context: Arc<C>,
    /// The Vello renderer instance.
    renderer: RefCell<Renderer>,
    text_renderer: RefCell<TextRenderer>,
    font_manager: RefCell<FontManager>,
    font_cache: RefCell<FontCache>,
}

impl<C: WgpuContextProvider + Send + Sync> VelloBackend<C> {
    pub fn new(context: Arc<C>) -> Result<Self> {
        let renderer = Renderer::new(context.device(), RendererOptions::default())?;

        Ok(Self {
            context,
            renderer: RefCell::new(renderer),
            text_renderer: RefCell::new(TextRenderer::new()),
            font_manager: RefCell::new(FontManager::new()),
            font_cache: RefCell::new(FontCache::new()),
        })
    }

    /// Takes a scene and renders it to the given surface.
    fn render_to_surface(&self, surface: &VelloSurface, scene: &Scene) -> Result<()> {
        // Retrieve the texture and view from our texture store
        let (_texture, texture_view) = self
            .context
            .get_texture(surface.texture_store_id)
            .expect("invalid texture id in VelloSurface");

        self.renderer.borrow_mut().render_to_texture(
            self.context.device(),
            self.context.queue(),
            scene,
            &texture_view,
            &RenderParams {
                base_color: Color::WHITE,
                width: surface.size.width,
                height: surface.size.height,
                antialiasing_method: vello::AaConfig::Msaa16,
            },
        )?;

        Ok(())
    }

    fn convert_browsing_context_to_scene(
        &self,
        text_renderer: &mut TextRenderer,
        font_manager: &mut FontManager,
        font_cache: &mut FontCache,
        ctx: &mut BrowsingContext,
    ) -> Result<Scene> {
        // Build a scene from your DisplayItems
        let vp = ctx.viewport();
        let offset_x = vp.x as f32;
        let offset_y = vp.y as f32;

        let mut scene = Scene::new();
        for item in ctx.render_list().items.iter() {
            match item {
                DisplayItem::Clear { color } => {
                    // full-frame clear
                    scene.fill(
                        Fill::NonZero,
                        Affine::IDENTITY,
                        Color::new([color.r, color.g, color.b, color.a]),
                        None,
                        &vello::kurbo::Rect::new(0.0, 0.0, vp.width as f64, vp.height as f64),
                    );
                }
                DisplayItem::Rect { x, y, w, h, color } => {
                    let x = *x - offset_x;
                    let y = *y - offset_y;
                    let w = *w;
                    let h = *h;
                    scene.fill(
                        Fill::NonZero,
                        Affine::IDENTITY,
                        Color::new([color.r, color.g, color.b, color.a]),
                        None,
                        &vello::kurbo::Rect::new(x as f64, y as f64, (x + w) as f64, (y + h) as f64),
                    );
                }
                DisplayItem::TextRun {
                    x,
                    y,
                    text,
                    size,
                    color,
                    max_width,
                } => {
                    let x = *x - offset_x;
                    let y = *y - offset_y;

                    let key = TextKey {
                        text: Arc::from(text.as_str()),
                        font_name: Arc::from("Comic Sans"),
                        font_size: size.ceil() as u32,
                        wrap: max_width.map(|mw| mw.ceil() as u32),
                        // wrap: Some(600),
                        align: 0,
                    };

                    text_renderer.draw(font_manager, font_cache, &mut scene, &key, x, y, (*color).into());
                }
            }
        }

        Ok(scene)
    }
}

impl<C: WgpuContextProvider + Send + Sync> RenderBackend for VelloBackend<C> {
    fn name(&self) -> &'static str {
        "vello"
    }

    fn create_surface(&self, size: SurfaceSize, _present: PresentMode) -> Result<Box<dyn ErasedSurface + Send>> {
        let texture_store_id =
            self.context
                .create_texture(size.width, size.height, wgpu::TextureFormat::Rgba8UnormSrgb);

        Ok(Box::new(VelloSurface {
            texture_store_id,
            size,
            frame_id: 1,
        }))
    }

    fn render(&self, ctx: &mut BrowsingContext, surface: &mut dyn ErasedSurface) -> Result<()> {
        // Downcast
        let s = surface
            .as_any_mut()
            .downcast_mut::<VelloSurface>()
            .ok_or_else(|| anyhow!("VelloBackend used with non-vello surface"))?;

        // Generate a scene which contains the gpu render commands
        let scene = {
            let mut tr = self.text_renderer.borrow_mut();
            let mut fm = self.font_manager.borrow_mut();
            let mut fc = self.font_cache.borrow_mut();
            self.convert_browsing_context_to_scene(&mut tr, &mut fm, &mut fc, ctx)?
        };

        // Render the scene to the surface
        self.render_to_surface(s, &scene)?;

        // Increment frame id, since we have rendered a new frame onto the surface
        s.frame_id = s.frame_id.wrapping_add(1);

        Ok(())
    }

    /// Takes a snapshot of the surface and returns it as an RGBA image
    fn snapshot(&self, _surface: &mut dyn ErasedSurface, _max_dim: u32) -> Result<RgbaImage> {
        Err(anyhow!("VelloBackend snapshot not implemented"))
    }

    /// Converts a surface into an external handle for sending to the compositor
    fn external_handle(&self, surface: &mut dyn ErasedSurface) -> Result<ExternalHandle> {
        let s = surface
            .as_any_mut()
            .downcast_mut::<VelloSurface>()
            .ok_or_else(|| anyhow!("VelloBackend used with non-vello surface in external_handle()"))?;

        Ok(ExternalHandle::WgpuTextureId {
            id: s.texture_store_id,
            width: s.size.width,
            height: s.size.height,
            format: GpuPixelFormat::Rgba8UnormSrgb,
            frame_id: s.frame_id,
        })
    }
}

/// A vello surface that wraps a wgpu texture.
struct VelloSurface {
    texture_store_id: u64,
    size: SurfaceSize,
    frame_id: u64,
}

impl ErasedSurface for VelloSurface {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn size(&self) -> SurfaceSize {
        self.size
    }
}
