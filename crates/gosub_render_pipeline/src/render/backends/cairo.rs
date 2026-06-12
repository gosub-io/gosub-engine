use crate::render::backend::{
    ErasedSurface, ExternalHandle, PixelFormat, PresentMode, RenderBackend, RgbaImage, SurfaceSize,
};
use crate::render::render_context::RenderContext;
use crate::render::render_list::DisplayItem;
use anyhow::{anyhow, Result};
use std::any::Any;

/// Device-pixel ratio shared between the GTK display thread and the render backends.
/// Set once from `area.scale_factor()` before any rendering begins.
pub static DEVICE_PIXEL_RATIO: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

/// Cairo backend for rendering using gtk4/cairo graphics library.
#[derive(Default)]
pub struct CairoBackend;

impl CairoBackend {
    pub fn new() -> Self {
        Self {}
    }
}

impl RenderBackend for CairoBackend {
    fn name(&self) -> &'static str {
        "cairo"
    }

    fn create_surface(&self, size: SurfaceSize, present: PresentMode) -> Result<Box<dyn ErasedSurface + Send>> {
        let dpr = DEVICE_PIXEL_RATIO.load(std::sync::atomic::Ordering::Relaxed);
        let physical = SurfaceSize {
            width: size.width * dpr,
            height: size.height * dpr,
        };
        Ok(Box::new(CairoSurface::new(physical, present)?))
    }

    #[allow(unsafe_code)] // Blit creates a cairo image surface over borrowed pixel data
    fn render(&self, ctx: &mut dyn RenderContext, surface: &mut dyn ErasedSurface) -> Result<()> {
        let s = surface
            .as_any_mut()
            .downcast_mut::<CairoSurface>()
            .ok_or_else(|| anyhow!("CairoBackend used with non-Cairo surface"))?;

        let vp = ctx.viewport();
        let dpr = DEVICE_PIXEL_RATIO.load(std::sync::atomic::Ordering::Relaxed) as f64;
        // All CSS-pixel coordinates are multiplied by DPR to get physical pixel positions.
        let offset_x = vp.x as f64 * dpr;
        let offset_y = vp.y as f64 * dpr;
        let size = s.size();

        s.with_ctx(|cr| {
            cr.rectangle(0.0, 0.0, size.width as f64, size.height as f64);
            cr.clip();

            let _ = cr.save();
            cr.translate(-offset_x, -offset_y);

            for item in ctx.render_list().items.iter() {
                match item {
                    DisplayItem::Clear { color } => {
                        cr.set_operator(cairo::Operator::Source);
                        cr.set_source_rgba(color.r as f64, color.g as f64, color.b as f64, color.a as f64);
                        _ = cr.paint();
                        cr.set_operator(cairo::Operator::Over);
                    }
                    DisplayItem::Rect { x, y, w, h, color } => {
                        cr.set_source_rgba(color.r as f64, color.g as f64, color.b as f64, color.a as f64);
                        cr.rectangle(*x as f64, *y as f64, *w as f64, *h as f64);
                        _ = cr.fill();
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
                        _ = cr.show_text(text);
                    }
                    DisplayItem::Blit { x, y, w, h, data, format } => {
                        let stride = (*w * 4) as i32;
                        let expected_len = (*h as usize) * (stride as usize);
                        if data.len() < expected_len {
                            log::warn!(
                                "CairoBackend: Blit data too short ({} < {}); skipping tile",
                                data.len(),
                                expected_len
                            );
                            continue;
                        }
                        // Cairo's ARgb32 wants [B, G, R, A]; convert (no-op when the tile is already
                        // in that order) so colors are correct whatever rasterizer produced the tile.
                        let data = format.to_argb32(data);
                        // SAFETY: `data` is borrowed for the duration of this closure;
                        // Cairo reads (never writes) source data.
                        let img_surface = unsafe {
                            cairo::ImageSurface::create_for_data_unsafe(
                                data.as_ptr() as *mut u8,
                                cairo::Format::ARgb32,
                                *w as i32,
                                *h as i32,
                                stride,
                            )
                        };
                        if let Ok(img_surface) = img_surface {
                            let pattern = cairo::SurfacePattern::create(&img_surface);
                            pattern.set_filter(cairo::Filter::Fast);
                            // x, y are CSS pixels; multiply by DPR to get physical position.
                            let phys_x = *x as f64 * dpr;
                            let phys_y = *y as f64 * dpr;
                            let mut matrix = cairo::Matrix::identity();
                            matrix.translate(-phys_x + offset_x, -phys_y + offset_y);
                            pattern.set_matrix(matrix);
                            cr.set_source(&pattern).unwrap_or(());
                            // w, h are already in physical pixels (tile surface dimensions).
                            cr.rectangle(phys_x, phys_y, *w as f64, *h as f64);
                            _ = cr.fill();
                        }
                    }
                }
            }

            let _ = cr.restore();
        })?;

        s.frame_id = s.frame_id.wrapping_add(1);
        Ok(())
    }

    fn snapshot(&self, surface: &mut dyn ErasedSurface, _max_dim: u32) -> Result<RgbaImage> {
        let s = surface
            .as_any_mut()
            .downcast_mut::<CairoSurface>()
            .ok_or_else(|| anyhow!("CairoBackend used with non-Cairo surface"))?;

        let pixels = s.pixels.as_ref().to_vec();
        Ok(RgbaImage::from_raw(
            pixels,
            s.size.width,
            s.size.height,
            s.stride as u32,
            PixelFormat::PreMulArgb32,
        ))
    }

    fn external_handle(&self, surface: &mut dyn ErasedSurface) -> Result<ExternalHandle> {
        let s = surface
            .as_any_mut()
            .downcast_mut::<CairoSurface>()
            .ok_or_else(|| anyhow!("CairoBackend used with non-Cairo surface"))?;

        if s.size.width == 0 || s.size.height == 0 || s.stride == 0 || s.pixels.is_empty() {
            return Ok(ExternalHandle::NullHandle {
                width: s.size.width,
                height: s.size.height,
                frame_id: s.frame_id,
            });
        }

        Ok(ExternalHandle::CpuPixelsOwned {
            width: s.size.width,
            height: s.size.height,
            stride: s.stride as u32,
            pixels: s.pixels.to_vec(),
            format: PixelFormat::PreMulArgb32,
        })
    }
}

pub struct CairoSurface {
    pixels: Box<[u8]>,
    size: SurfaceSize,
    stride: i32,
    #[allow(unused)]
    present: PresentMode,
    frame_id: u64,
}

impl CairoSurface {
    fn new(size: SurfaceSize, present: PresentMode) -> Result<Self> {
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

    #[allow(unsafe_code)] // cairo draws directly into our pixel buffer via a raw pointer
    pub fn with_ctx<R>(&mut self, f: impl FnOnce(&cairo::Context) -> R) -> Result<R> {
        let w = self.size.width as i32;
        let h = self.size.height as i32;
        let stride = self.stride;

        // SAFETY: `ptr` stays valid for the surface's lifetime — `self.pixels` is not
        // touched until the surface is flushed and dropped at the end of this call.
        let ptr = self.pixels.as_mut_ptr();
        let surface = unsafe { cairo::ImageSurface::create_for_data_unsafe(ptr, cairo::Format::ARgb32, w, h, stride)? };

        let cr = cairo::Context::new(&surface)?;
        let out = f(&cr);
        surface.flush();
        Ok(out)
    }

    #[inline]
    pub fn stride(&self) -> i32 {
        self.stride
    }

    pub fn pixels_borrowed(&self) -> (&[u8], u32, u32, u32) {
        (&self.pixels, self.size.width, self.size.height, self.stride as u32)
    }
}

impl ErasedSurface for CairoSurface {
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
