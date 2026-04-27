use gosub_engine_api::render::backend::{
    ErasedSurface, ExternalHandle, PixelFormat, PresentMode, RenderBackend as EngineRenderBackend, RgbaImage,
    SurfaceSize,
};
use gosub_engine_api::render::DisplayItem;
use gosub_engine_api::BrowsingContext;
use std::any::Any;
use std::ptr::NonNull;

use crate::CairoBackend;

/// Off-screen Cairo surface used by the new `gosub_engine_api::RenderBackend` path.
pub struct OffscreenCairoSurface {
    pixels: Box<[u8]>,
    size: SurfaceSize,
    stride: i32,
    #[allow(unused)]
    present: PresentMode,
    frame_id: u64,
}

impl OffscreenCairoSurface {
    fn new(size: SurfaceSize, present: PresentMode) -> anyhow::Result<Self> {
        let stride = cairo::Format::ARgb32
            .stride_for_width(size.width)
            .unwrap_or((size.width * 4) as i32);

        let pixels: Box<[u8]> = vec![0u8; (size.height as usize) * (stride as usize)].into_boxed_slice();

        Ok(Self {
            pixels,
            size,
            stride,
            present,
            frame_id: 0,
        })
    }

    fn with_ctx<R>(&mut self, f: impl FnOnce(&cairo::Context) -> R) -> anyhow::Result<R> {
        let w = self.size.width as i32;
        let h = self.size.height as i32;
        let ptr = self.pixels.as_mut_ptr();

        let surface =
            unsafe { cairo::ImageSurface::create_for_data_unsafe(ptr, cairo::Format::ARgb32, w, h, self.stride)? };
        let cr = cairo::Context::new(&surface)?;
        let out = f(&cr);
        surface.flush();
        Ok(out)
    }
}

impl ErasedSurface for OffscreenCairoSurface {
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

impl EngineRenderBackend for CairoBackend {
    fn name(&self) -> &'static str {
        "cairo"
    }

    fn create_surface(&self, size: SurfaceSize, present: PresentMode) -> anyhow::Result<Box<dyn ErasedSurface + Send>> {
        Ok(Box::new(OffscreenCairoSurface::new(size, present)?))
    }

    fn render(&self, ctx: &mut BrowsingContext, surface: &mut dyn ErasedSurface) -> anyhow::Result<()> {
        let s = surface
            .as_any_mut()
            .downcast_mut::<OffscreenCairoSurface>()
            .expect("CairoBackend::render called with non-OffscreenCairoSurface");

        let vp = ctx.viewport();
        let offset_x = vp.x as f64;
        let offset_y = vp.y as f64;
        let sz = s.size();

        s.with_ctx(|cr| {
            cr.rectangle(0.0, 0.0, sz.width as f64, sz.height as f64);
            cr.clip();

            let _ = cr.save();
            cr.translate(-offset_x, -offset_y);

            for item in ctx.render_list().items.iter() {
                match item {
                    DisplayItem::Clear { color } => {
                        cr.set_operator(cairo::Operator::Source);
                        cr.set_source_rgba(color.r as f64, color.g as f64, color.b as f64, color.a as f64);
                        let _ = cr.paint();
                        cr.set_operator(cairo::Operator::Over);
                    }
                    DisplayItem::Rect { x, y, w, h, color } => {
                        cr.set_source_rgba(color.r as f64, color.g as f64, color.b as f64, color.a as f64);
                        cr.rectangle(*x as f64, *y as f64, *w as f64, *h as f64);
                        let _ = cr.fill();
                    }
                    DisplayItem::TextRun {
                        x,
                        y,
                        text,
                        size,
                        color,
                        ..
                    } => {
                        cr.set_source_rgba(color.r as f64, color.g as f64, color.b as f64, color.a as f64);
                        cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Normal);
                        cr.set_font_size(*size as f64);
                        cr.move_to(*x as f64, *y as f64);
                        let _ = cr.show_text(text);
                    }
                }
            }

            let _ = cr.restore();
        })?;

        s.frame_id = s.frame_id.wrapping_add(1);
        Ok(())
    }

    fn snapshot(&self, surface: &mut dyn ErasedSurface, _max_dim: u32) -> anyhow::Result<RgbaImage> {
        let s = surface
            .as_any_mut()
            .downcast_mut::<OffscreenCairoSurface>()
            .ok_or_else(|| anyhow::anyhow!("CairoBackend::snapshot called with non-OffscreenCairoSurface"))?;

        Ok(RgbaImage::from_raw(
            s.pixels.as_ref().to_vec(),
            s.size.width,
            s.size.height,
            s.stride as u32,
            PixelFormat::PreMulArgb32,
        ))
    }

    fn external_handle(&self, surface: &mut dyn ErasedSurface) -> anyhow::Result<ExternalHandle> {
        let s = surface
            .as_any_mut()
            .downcast_mut::<OffscreenCairoSurface>()
            .ok_or_else(|| anyhow::anyhow!("CairoBackend::external_handle called with non-OffscreenCairoSurface"))?;

        if s.size.width == 0 || s.size.height == 0 || s.stride == 0 || s.pixels.is_empty() {
            return Ok(ExternalHandle::NullHandle {
                width: s.size.width,
                height: s.size.height,
                frame_id: s.frame_id,
            });
        }

        let ptr = NonNull::new(s.pixels.as_mut_ptr())
            .ok_or_else(|| anyhow::anyhow!("OffscreenCairoSurface has null pixel buffer"))?;

        Ok(ExternalHandle::CpuPixelsPtr {
            width: s.size.width,
            height: s.size.height,
            stride: s.stride as u32,
            pixel_buf: ptr,
        })
    }
}
