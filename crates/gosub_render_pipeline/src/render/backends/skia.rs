use crate::render::backend::{
    ErasedSurface, ExternalHandle, PixelFormat, PresentMode, RenderBackend, RgbaImage, SurfaceSize,
};
use crate::render::render_context::RenderContext;
use crate::render::render_list::DisplayItem;
use anyhow::{anyhow, Result};
use skia_safe::{Color4f, Font, FontMgr, FontStyle, Paint, Rect};
use std::any::Any;

thread_local! {
    static FONT_MGR: FontMgr = FontMgr::new();
}

#[derive(Default)]
pub struct SkiaBackend;

impl SkiaBackend {
    pub fn new() -> Self {
        Self {}
    }
}

impl RenderBackend for SkiaBackend {
    fn name(&self) -> &'static str {
        "skia"
    }

    fn create_surface(&self, size: SurfaceSize, present: PresentMode) -> Result<Box<dyn ErasedSurface + Send>> {
        Ok(Box::new(SkiaSurface::new(size, present)?))
    }

    fn render(&self, ctx: &mut dyn RenderContext, surface: &mut dyn ErasedSurface) -> Result<()> {
        let s = surface
            .as_any_mut()
            .downcast_mut::<SkiaSurface>()
            .ok_or_else(|| anyhow!("SkiaBackend used with non-Skia surface"))?;

        let vp = ctx.viewport();
        let offset_x = vp.x as f32;
        let offset_y = vp.y as f32;
        let clip = Rect::new(0.0, 0.0, s.size.width as f32, s.size.height as f32);
        let items: Vec<DisplayItem> = ctx.render_list().items.to_vec();

        s.with_canvas(|canvas| {
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
                    DisplayItem::TextRun {
                        x,
                        y,
                        text,
                        size,
                        color,
                        ..
                    } => {
                        let Some(typeface) = FONT_MGR.with(|fm| {
                            fm.legacy_make_typeface(None, FontStyle::normal())
                                .or_else(|| fm.legacy_make_typeface("sans-serif", FontStyle::normal()))
                        }) else {
                            log::warn!("SkiaBackend: no typeface available; skipping text run");
                            continue;
                        };
                        let font = Font::new(typeface, *size);
                        let mut paint = Paint::new(to_color4f(color), None);
                        paint.set_anti_alias(true);
                        canvas.draw_str(text.as_str(), (*x, *y), &font, &paint);
                    }
                    DisplayItem::Blit { x, y, w, h, data, format } => {
                        let stride = (*w * 4) as usize;
                        let expected = *h as usize * stride;
                        if data.len() < expected {
                            log::warn!("SkiaBackend: Blit data too short ({} < {})", data.len(), expected);
                            continue;
                        }
                        // BGRA8888 wants [B, G, R, A]; convert (no-op when already in that order).
                        let data = format.to_argb32(data);
                        let info = skia_safe::ImageInfo::new(
                            skia_safe::ISize::new(*w as i32, *h as i32),
                            skia_safe::ColorType::BGRA8888,
                            skia_safe::AlphaType::Premul,
                            None,
                        );
                        if let Some(image) =
                            skia_safe::images::raster_from_data(&info, skia_safe::Data::new_copy(&data), stride)
                        {
                            canvas.draw_image(&image, (*x, *y), None);
                        }
                    }
                }
            }

            canvas.restore();
        });

        s.frame_id = s.frame_id.wrapping_add(1);
        Ok(())
    }

    fn snapshot(&self, surface: &mut dyn ErasedSurface, _max_dim: u32) -> Result<RgbaImage> {
        let s = surface
            .as_any_mut()
            .downcast_mut::<SkiaSurface>()
            .ok_or_else(|| anyhow!("SkiaBackend used with non-Skia surface"))?;

        Ok(RgbaImage::from_raw(
            s.pixels.clone(),
            s.size.width,
            s.size.height,
            s.size.width * 4,
            PixelFormat::PreMulArgb32,
        ))
    }

    fn external_handle(&self, surface: &mut dyn ErasedSurface) -> Result<ExternalHandle> {
        let s = surface
            .as_any_mut()
            .downcast_mut::<SkiaSurface>()
            .ok_or_else(|| anyhow!("SkiaBackend used with non-Skia surface"))?;

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

pub struct SkiaSurface {
    size: SurfaceSize,
    pixels: Vec<u8>,
    #[allow(dead_code)]
    present: PresentMode,
    frame_id: u64,
}

impl SkiaSurface {
    fn new(size: SurfaceSize, present: PresentMode) -> Result<Self> {
        let pixels = vec![0u8; size.width as usize * size.height as usize * 4];
        Ok(Self {
            size,
            pixels,
            present,
            frame_id: 0,
        })
    }

    fn with_canvas(&mut self, f: impl FnOnce(&skia_safe::Canvas)) {
        let Some(mut surface) = skia_safe::surfaces::raster_n32_premul(skia_safe::ISize::new(
            self.size.width as i32,
            self.size.height as i32,
        )) else {
            log::error!(
                "SkiaBackend: failed to create raster surface {}x{}",
                self.size.width,
                self.size.height
            );
            return;
        };

        f(surface.canvas());

        if let Some(peek) = surface.canvas().peek_pixels() {
            if let Some(bytes) = peek.bytes() {
                self.pixels = bytes.to_vec();
            }
        }
    }
}

impl ErasedSurface for SkiaSurface {
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

#[inline]
fn to_color4f(c: &crate::render::render_list::Color) -> Color4f {
    Color4f::new(c.r, c.g, c.b, c.a)
}
