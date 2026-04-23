use crate::engine::BrowsingContext;
use crate::render::backend::{ErasedSurface, ExternalHandle, PixelFormat, PresentMode, RenderBackend, RgbaImage, SurfaceSize};
use crate::render::DisplayItem;
use anyhow::{anyhow, Result};
use std::any::Any;
use std::ptr::NonNull;

/// Cairo backend for rendering using gtk4/cairo graphics library.
pub struct CairoBackend;

impl CairoBackend {
    /// Creates a new instance of the Cairo backend.
    pub fn new() -> Self {
        Self {}
    }
}

impl RenderBackend for CairoBackend {
    /// Returns the name of the backend.
    fn name(&self) -> &'static str {
        "cairo"
    }

    /// Will create a new Cairo surface with the given size and present mode.
    fn create_surface(&self, size: SurfaceSize, present: PresentMode) -> Result<Box<dyn ErasedSurface + Send>> {
        Ok(Box::new(CairoSurface::new(size, present)?))
    }

    /// Renders a surface by getting the DisplayItems from the browsing context and rendering them
    /// onto the ErasedSurface
    fn render(&self, ctx: &mut BrowsingContext, surface: &mut dyn ErasedSurface) -> Result<()> {
        // Ensure the surface is a CairoSurface.
        let s = surface
            .as_any_mut()
            .downcast_mut::<CairoSurface>()
            .expect("CairoBackend used with non-Cairo surface");

        // Viewport offset. We must take this into account when rendering items.
        let vp = ctx.viewport();
        let offset_x = vp.x as f64;
        let offset_y = vp.y as f64;
        let size = s.size();

        // Get the cairo context (CR) from the surface.
        s.with_ctx(|cr| {
            cr.rectangle(0.0, 0.0, size.width as f64, size.height as f64);
            cr.clip();

            let _ = cr.save();
            cr.translate(-offset_x, -offset_y);

            for item in ctx.render_list().items.iter() {
                match item {
                    DisplayItem::Clear { color } => {
                        // Clear the surface with the specified color.
                        cr.set_operator(cairo::Operator::Source);
                        cr.set_source_rgba(
                            color.r as f64,
                            color.g as f64,
                            color.b as f64,
                            color.a as f64,
                        );
                        _ = cr.paint();
                        cr.set_operator(cairo::Operator::Over);
                    }
                    DisplayItem::Rect { x, y, w, h, color } => {
                        // Draw a rectangle with the specified color.
                        cr.set_source_rgba(
                            color.r as f64,
                            color.g as f64,
                            color.b as f64,
                            color.a as f64,
                        );
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
                        // Draw text at the specified position with the specified size and color.
                        cr.set_source_rgba(
                            color.r as f64,
                            color.g as f64,
                            color.b as f64,
                            color.a as f64,
                        );
                        cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Normal);
                        cr.set_font_size(*size as f64);
                        cr.move_to(*x as f64, *y as f64);
                        _ = cr.show_text(text);
                    }
                }
            }

            let _ = cr.restore();
        })?;

        s.frame_id = s.frame_id.wrapping_add(1);
        Ok(())
    }

    /// Generates a snapshot of the surface as a small RGBA8 image.
    fn snapshot(&self, surface: &mut dyn ErasedSurface, _max_dim: u32) -> Result<RgbaImage> {
        let s = surface
            .as_any_mut()
            .downcast_mut::<CairoSurface>()
            .ok_or_else(|| anyhow!("CairoBackend used with non-Cairo surface"))?;

        let pixels = s.pixels.as_ref().to_vec();
        Ok(RgbaImage::from_raw(pixels, s.size.width, s.size.height, s.stride as u32, PixelFormat::PreMulArgb32))
    }

    fn external_handle(&self, surface: &mut dyn ErasedSurface) -> Result<ExternalHandle> {
        let s = surface
            .as_any_mut()
            .downcast_mut::<CairoSurface>()
            .ok_or_else(|| anyhow!("CairoBackend used with non-Cairo surface"))?;

        if s.size.width == 0 || s.size.height == 0 || s.stride == 0 || s.pixels.is_empty() {
            return Ok(ExternalHandle::NullHandle { width: s.size.width, height: s.size.height, frame_id: s.frame_id });
        }

        let ptr = NonNull::new(s.pixels.as_mut_ptr()).ok_or_else(|| anyhow!("CairoSurface has null pixel buffer"))?;
        Ok(ExternalHandle::CpuPixelsPtr {
            width: s.size.width,
            height: s.size.height,
            stride: s.stride as u32,
            pixel_buf: ptr,
        })
    }
}

/// A Cairo surface that can be used for rendering.
pub struct CairoSurface {
    /// The image buffer
    pixels: Box<[u8]>,
    /// Size of the surface in pixels.
    size: SurfaceSize,
    /// Stride of the surface in bytes.
    stride: i32,
    /// Present mode for the surface.
    #[allow(unused)]
    present: PresentMode,
    /// Frame ID for the surface, used to track rendering frames.
    frame_id: u64,
}

impl CairoSurface {
    fn new(size: SurfaceSize, present: PresentMode) -> Result<Self> {
        let stride = cairo::Format::ARgb32
            .stride_for_width(size.width)
            .unwrap_or((size.width * 4) as i32);

        // Allocate a buffer large enough for the surface to be mapped on top.
        let pixels: Box<[u8]> = vec![0u8; (size.height as usize) * (stride as usize)].into_boxed_slice();

        Ok(Self {
            pixels,
            size,
            stride,
            present,
            frame_id: 0,
        })
    }

    pub fn with_ctx<R>(&mut self, f: impl FnOnce(&cairo::Context) -> R) -> Result<R> {
        let w = self.size.width as i32;
        let h = self.size.height as i32;
        let stride = self.stride;

        let ptr = self .pixels.as_mut_ptr();
        let surface = unsafe {
            cairo::ImageSurface::create_for_data_unsafe(
                ptr,
                cairo::Format::ARgb32,
                w,
                h,
                stride,
            )?
        };

        let cr = cairo::Context::new(&surface)?;
        let out = f(&cr);
        surface.flush();
        Ok(out)
    }

    /// Returns the stride of the surface in bytes.
    #[inline]
    pub fn stride(&self) -> i32 {
        self.stride
    }

    /// Cheap read-only borrow of the pixels (no copy).
    /// Lifetime is tied to &self, and you must not draw while holding this slice.
    pub fn pixels_borrowed(&self) -> (&[u8], u32, u32, u32) {
        // self.flush();
        (
            &self.pixels,
            self.size.width,
            self.size.height,
            self.stride as u32,
        )
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
