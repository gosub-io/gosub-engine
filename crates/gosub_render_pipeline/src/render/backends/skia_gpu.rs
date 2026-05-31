use crate::render::backend::{
    ErasedSurface, ExternalHandle, PixelFormat, PresentMode, RenderBackend, RgbaImage, SurfaceSize,
};
use crate::render::render_context::RenderContext;
use crate::render::render_list::DisplayItem;
use anyhow::{anyhow, Result};
use parking_lot::Mutex;
use skia_safe::gpu::ganesh::surface_ganesh::render_target as gpu_render_target;
use skia_safe::gpu::{Budgeted, DirectContext, SurfaceOrigin};
use skia_safe::{Color4f, Font, FontMgr, FontStyle, ImageInfo, Paint, Rect, Surface};
use std::any::Any;
use std::sync::Arc;

/// Trait for providing an OpenGL context to `SkiaGpuBackend`.
pub trait GlContextProvider: Send + Sync {
    /// Make the GL context current on the calling thread.
    fn make_current(&self);
    /// Return the address of an OpenGL function, or null if not available.
    fn get_proc_address(&self, name: &str) -> *const std::ffi::c_void;
}

/// Wraps `DirectContext` to allow crossing thread boundaries.
///
/// SAFETY: `GlContextProvider::make_current()` is called before every GPU operation,
/// ensuring the GL context is current on whichever thread the engine uses.
struct SendDirectContext(DirectContext);
unsafe impl Send for SendDirectContext {}
unsafe impl Sync for SendDirectContext {}

pub struct SkiaGpuBackend<C: GlContextProvider> {
    context: Arc<C>,
    direct_context: Mutex<SendDirectContext>,
}

impl<C: GlContextProvider> SkiaGpuBackend<C> {
    pub fn new(context: Arc<C>) -> Result<Self> {
        context.make_current();

        let interface = skia_safe::gpu::gl::Interface::new_load_with(|name| {
            context.get_proc_address(name)
        })
        .ok_or_else(|| anyhow!("Failed to create Skia GL interface — no GL functions found"))?;

        let direct_context = DirectContext::new_gl(interface, None)
            .ok_or_else(|| anyhow!("Failed to create Skia GL DirectContext — GL context must be current"))?;

        Ok(Self {
            context,
            direct_context: Mutex::new(SendDirectContext(direct_context)),
        })
    }
}

impl<C: GlContextProvider + Send + Sync + 'static> RenderBackend for SkiaGpuBackend<C> {
    fn name(&self) -> &'static str {
        "skia-gpu"
    }

    fn create_surface(&self, size: SurfaceSize, present: PresentMode) -> Result<Box<dyn ErasedSurface + Send>> {
        Ok(Box::new(SkiaGpuSurface::new(size, present)))
    }

    fn render(&self, ctx: &mut dyn RenderContext, surface: &mut dyn ErasedSurface) -> Result<()> {
        let s = surface
            .as_any_mut()
            .downcast_mut::<SkiaGpuSurface>()
            .ok_or_else(|| anyhow!("SkiaGpuBackend used with non-SkiaGpu surface"))?;

        if s.size.width == 0 || s.size.height == 0 {
            return Ok(());
        }

        self.context.make_current();

        let vp = ctx.viewport();
        let offset_x = vp.x as f32;
        let offset_y = vp.y as f32;
        let clip = Rect::new(0.0, 0.0, s.size.width as f32, s.size.height as f32);
        let items: Vec<DisplayItem> = ctx.render_list().items.to_vec();

        let image_info = ImageInfo::new_n32_premul(
            (s.size.width as i32, s.size.height as i32),
            None,
        );

        let mut dc = self.direct_context.lock();

        let Some(mut gpu_surface): Option<Surface> = gpu_render_target(
            &mut dc.0,
            Budgeted::No,
            &image_info,
            None,
            SurfaceOrigin::TopLeft,
            None,
            None,
            None,
        ) else {
            return Err(anyhow!("Failed to create Skia GPU render target"));
        };

        {
            let canvas = gpu_surface.canvas();
            canvas.clip_rect(clip, None, None);
            canvas.save();
            canvas.translate((-offset_x, -offset_y));

            for item in &items {
                match item {
                    DisplayItem::Clear { color } => {
                        canvas.clear(to_color4f(color));
                    }
                    DisplayItem::Rect { x, y, w, h, color } => {
                        let mut paint = Paint::new(to_color4f(color), None);
                        paint.set_anti_alias(true);
                        canvas.draw_rect(Rect::new(*x, *y, x + w, y + h), &paint);
                    }
                    DisplayItem::TextRun { x, y, text, size, color, .. } => {
                        let typeface = FontMgr::new()
                            .legacy_make_typeface(None, FontStyle::normal())
                            .unwrap_or_else(|| {
                                FontMgr::new()
                                    .legacy_make_typeface("sans-serif", FontStyle::normal())
                                    .expect("no typeface")
                            });
                        let font = Font::new(typeface, *size);
                        let mut paint = Paint::new(to_color4f(color), None);
                        paint.set_anti_alias(true);
                        canvas.draw_str(text.as_str(), (*x, *y), &font, &paint);
                    }
                    DisplayItem::Blit { x, y, w, h, data } => {
                        let stride = (*w * 4) as usize;
                        if data.len() < *h as usize * stride {
                            log::warn!("SkiaGpuBackend: Blit data too short");
                            continue;
                        }
                        let info = ImageInfo::new(
                            (*w as i32, *h as i32),
                            skia_safe::ColorType::BGRA8888,
                            skia_safe::AlphaType::Premul,
                            None,
                        );
                        if let Some(image) = skia_safe::images::raster_from_data(
                            &info,
                            skia_safe::Data::new_copy(data),
                            stride,
                        ) {
                            canvas.draw_image(&image, (*x, *y), None);
                        }
                    }
                }
            }

            canvas.restore();
        }

        // Flush GPU commands and read pixels back to CPU buffer.
        dc.0.flush_surface(&mut gpu_surface);
        dc.0.submit(skia_safe::gpu::SyncCpu::Yes);

        let stride = s.size.width as usize * 4;
        s.pixels.resize(s.size.height as usize * stride, 0);
        gpu_surface.read_pixels(&image_info, &mut s.pixels, stride, (0, 0));

        s.frame_id = s.frame_id.wrapping_add(1);
        Ok(())
    }

    fn snapshot(&self, surface: &mut dyn ErasedSurface, _max_dim: u32) -> Result<RgbaImage> {
        let s = surface
            .as_any_mut()
            .downcast_mut::<SkiaGpuSurface>()
            .ok_or_else(|| anyhow!("SkiaGpuBackend used with non-SkiaGpu surface"))?;
        Ok(RgbaImage::from_raw(
            s.pixels.clone(),
            s.size.width,
            s.size.height,
            (s.size.width * 4) as u32,
            PixelFormat::PreMulArgb32,
        ))
    }

    fn external_handle(&self, surface: &mut dyn ErasedSurface) -> Result<ExternalHandle> {
        let s = surface
            .as_any_mut()
            .downcast_mut::<SkiaGpuSurface>()
            .ok_or_else(|| anyhow!("SkiaGpuBackend used with non-SkiaGpu surface"))?;

        if s.size.width == 0 || s.size.height == 0 || s.pixels.is_empty() {
            return Ok(ExternalHandle::NullHandle {
                width: s.size.width,
                height: s.size.height,
                frame_id: s.frame_id,
            });
        }

        Ok(ExternalHandle::CpuPixelsOwned {
            width: s.size.width,
            height: s.size.height,
            stride: s.size.width * 4,
            pixels: s.pixels.clone(),
            format: PixelFormat::PreMulArgb32,
        })
    }
}

// ── Surface ───────────────────────────────────────────────────────────────────

pub struct SkiaGpuSurface {
    size: SurfaceSize,
    pixels: Vec<u8>,
    #[allow(dead_code)]
    present: PresentMode,
    frame_id: u64,
}

impl SkiaGpuSurface {
    fn new(size: SurfaceSize, present: PresentMode) -> Self {
        Self { size, pixels: Vec::new(), present, frame_id: 0 }
    }
}

impl ErasedSurface for SkiaGpuSurface {
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
    fn size(&self) -> SurfaceSize { self.size }
}

#[inline]
pub fn to_color4f(c: &crate::render::render_list::Color) -> Color4f {
    Color4f::new(c.r, c.g, c.b, c.a)
}

// ── Deferred framebuffer backend (for GTK4 GLArea) ───────────────────────────

/// A frame captured from the engine, ready to be drawn in a GL context callback.
#[derive(Clone)]
pub struct PendingFrame {
    pub items: Vec<DisplayItem>,
    pub offset_x: f32,
    pub offset_y: f32,
}

/// Backend that stores the display list for rendering in a GTK4 GLArea callback.
///
/// The engine calls `render()` on its thread — this just saves the display list.
/// The actual GPU rendering happens in `GLArea::connect_render` on the GTK main
/// thread, where the GL context is current.  The result is written directly into
/// GTK4's framebuffer — no CPU readback.
pub struct SkiaGpuDirectFbBackend {
    pub pending: Arc<Mutex<Option<PendingFrame>>>,
}

impl SkiaGpuDirectFbBackend {
    pub fn new() -> Self {
        Self { pending: Arc::new(Mutex::new(None)) }
    }
}

impl Default for SkiaGpuDirectFbBackend {
    fn default() -> Self { Self::new() }
}

impl RenderBackend for SkiaGpuDirectFbBackend {
    fn name(&self) -> &'static str { "skia-gpu-direct-fb" }

    fn create_surface(&self, size: SurfaceSize, present: PresentMode) -> Result<Box<dyn ErasedSurface + Send>> {
        Ok(Box::new(SkiaDirectFbSurface { size, frame_id: 0, present }))
    }

    fn render(&self, ctx: &mut dyn RenderContext, surface: &mut dyn ErasedSurface) -> Result<()> {
        let s = surface
            .as_any_mut()
            .downcast_mut::<SkiaDirectFbSurface>()
            .ok_or_else(|| anyhow!("SkiaGpuDirectFbBackend used with wrong surface type"))?;

        let vp = ctx.viewport();
        *self.pending.lock() = Some(PendingFrame {
            items: ctx.render_list().items.to_vec(),
            offset_x: vp.x as f32,
            offset_y: vp.y as f32,
        });
        s.frame_id = s.frame_id.wrapping_add(1);
        Ok(())
    }

    fn snapshot(&self, _surface: &mut dyn ErasedSurface, _max_dim: u32) -> Result<RgbaImage> {
        Err(anyhow!("SkiaGpuDirectFbBackend: snapshot not supported (frame is on GPU)"))
    }

    fn external_handle(&self, surface: &mut dyn ErasedSurface) -> Result<ExternalHandle> {
        let s = surface
            .as_any_mut()
            .downcast_mut::<SkiaDirectFbSurface>()
            .ok_or_else(|| anyhow!("SkiaGpuDirectFbBackend used with wrong surface type"))?;
        Ok(ExternalHandle::GlFramebufferRendered { frame_id: s.frame_id })
    }
}

struct SkiaDirectFbSurface {
    size: SurfaceSize,
    frame_id: u64,
    #[allow(dead_code)]
    present: PresentMode,
}

impl ErasedSurface for SkiaDirectFbSurface {
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
    fn size(&self) -> SurfaceSize { self.size }
}
