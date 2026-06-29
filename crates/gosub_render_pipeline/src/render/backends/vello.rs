use crate::render::backend::GpuPixelFormat;
use crate::render::backend::{ErasedSurface, ExternalHandle, PresentMode, RenderBackend, RgbaImage, SurfaceSize};
use crate::render::backends::vello::font_cache::FontCache;
use crate::render::backends::vello::font_manager::FontManager;
use crate::render::backends::vello::text_renderer::{TextKey, TextRenderer};
use crate::render::render_context::RenderContext;
use crate::render::render_list::DisplayItem;
use anyhow::{anyhow, Result};
use gosub_fontmanager::ParleyFontSystem;
use parking_lot::Mutex;
use parley::FontContext;
use std::any::Any;
use std::sync::Arc;
use vello::kurbo::{Affine, Vec2};
use vello::peniko::{Color, Fill, ImageAlphaType, ImageData, ImageFormat};
use vello::wgpu;
use vello::{RenderParams, Renderer, RendererOptions, Scene};

mod font_cache;
mod font_manager;
mod text_renderer;

pub trait WgpuContextProvider {
    fn device(&self) -> &wgpu::Device;
    fn queue(&self) -> &wgpu::Queue;
    fn device_arc(&self) -> Arc<wgpu::Device>;
    fn queue_arc(&self) -> Arc<wgpu::Queue>;
    fn create_texture(&self, width: u32, height: u32, format: wgpu::TextureFormat) -> u64;
    fn get_texture(&self, id: u64) -> Option<(wgpu::Texture, wgpu::TextureView)>;
    fn remove_texture(&self, id: u64);
}

/// Shareable wgpu resources used by both the display backend and the stage-6 rasterizer.
pub struct WgpuResources {
    pub device: Arc<wgpu::Device>,
    pub queue: Arc<wgpu::Queue>,
    pub renderer: Mutex<Renderer>,
}

pub struct VelloBackend<C: WgpuContextProvider + Send + Sync> {
    context: Arc<C>,
    resources: Arc<WgpuResources>,
    text_renderer: Mutex<TextRenderer>,
    font_manager: Mutex<FontManager>,
    font_cache: Mutex<FontCache>,
    /// Shared font system. Holds the Parley font collection so that all text
    /// shaping in this backend uses a single, consistent font discovery context.
    font_system: Arc<Mutex<ParleyFontSystem>>,
}

impl<C: WgpuContextProvider + Send + Sync> VelloBackend<C> {
    pub fn new(context: Arc<C>) -> Result<Self> {
        let renderer = Renderer::new(context.device(), RendererOptions::default())?;
        let resources = Arc::new(WgpuResources {
            device: context.device_arc(),
            queue: context.queue_arc(),
            renderer: Mutex::new(renderer),
        });

        Ok(Self {
            context,
            resources,
            text_renderer: Mutex::new(TextRenderer::new()),
            font_manager: Mutex::new(FontManager::new()),
            font_cache: Mutex::new(FontCache::new()),
            font_system: Arc::new(Mutex::new(ParleyFontSystem::new())),
        })
    }

    /// Expose the font system so callers can share it with `TaffyLayouter` or
    /// `VelloRasterizer` to ensure consistent font discovery across layout and render.
    pub fn font_system(&self) -> Arc<Mutex<ParleyFontSystem>> {
        Arc::clone(&self.font_system)
    }

    /// Returns the shared wgpu resources (device, queue, renderer) for use by the tile rasterizer.
    pub fn wgpu_resources(&self) -> Arc<WgpuResources> {
        Arc::clone(&self.resources)
    }

    fn render_to_surface(&self, surface: &VelloSurface, scene: &Scene) -> Result<()> {
        let (_texture, texture_view) = self
            .context
            .get_texture(surface.texture_store_id)
            .ok_or_else(|| anyhow!("invalid texture id in VelloSurface"))?;

        self.resources.renderer.lock().render_to_texture(
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

    fn build_scene(
        &self,
        text_renderer: &mut TextRenderer,
        font_manager: &mut FontManager,
        font_cache: &mut FontCache,
        font_cx: &mut FontContext,
        ctx: &mut dyn RenderContext,
    ) -> Result<Scene> {
        let vp = ctx.viewport();
        let offset_x = vp.x as f32;
        let offset_y = vp.y as f32;

        let mut scene = Scene::new();
        for item in ctx.render_list().items.iter() {
            match item {
                DisplayItem::Clear { color } => {
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
                        align: 0,
                    };

                    text_renderer.draw(
                        font_manager,
                        font_cache,
                        font_cx,
                        &mut scene,
                        &key,
                        x,
                        y,
                        (*color).into(),
                    );
                }
                DisplayItem::Blit {
                    x,
                    y,
                    w,
                    h,
                    data,
                    format,
                } => {
                    // peniko ImageFormat::Rgba8 expects [R, G, B, A]. The tile may be premultiplied
                    // ARGB32 (Cairo/Skia, [B, G, R, A]) or already RGBA (Vello); `to_rgba` swaps only
                    // when needed, so colors are correct regardless of which rasterizer produced it.
                    let rgba = format.to_rgba(data).into_owned();
                    let blob = vello::peniko::Blob::<u8>::new(Arc::new(rgba));
                    let image = ImageData {
                        data: blob,
                        format: ImageFormat::Rgba8,
                        alpha_type: ImageAlphaType::AlphaPremultiplied,
                        width: *w,
                        height: *h,
                    };
                    let ax = *x as f64 - offset_x as f64;
                    let ay = *y as f64 - offset_y as f64;
                    scene.draw_image(&image, Affine::translate(Vec2::new(ax, ay)));
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
        let texture_store_id = self
            .context
            .create_texture(size.width, size.height, wgpu::TextureFormat::Rgba8Unorm);

        Ok(Box::new(VelloSurface {
            texture_store_id,
            size,
            frame_id: 1,
        }))
    }

    fn render(&self, ctx: &mut dyn RenderContext, surface: &mut dyn ErasedSurface) -> Result<()> {
        let s = surface
            .as_any_mut()
            .downcast_mut::<VelloSurface>()
            .ok_or_else(|| anyhow!("VelloBackend used with non-vello surface"))?;

        let scene = {
            let mut tr = self.text_renderer.lock();
            let mut fm = self.font_manager.lock();
            let mut fc = self.font_cache.lock();
            let mut fs = self.font_system.lock();
            let font_cx = fs.font_cx_mut();
            self.build_scene(&mut tr, &mut fm, &mut fc, font_cx, ctx)?
        };

        self.render_to_surface(s, &scene)?;
        s.frame_id = s.frame_id.wrapping_add(1);

        Ok(())
    }

    fn snapshot(&self, _surface: &mut dyn ErasedSurface, _max_dim: u32) -> Result<RgbaImage> {
        Err(anyhow!("VelloBackend snapshot not implemented"))
    }

    fn wgpu_resources(&self) -> Option<Arc<WgpuResources>> {
        Some(Arc::clone(&self.resources))
    }

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
