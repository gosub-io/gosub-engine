use crate::engine::BrowsingContext;
use crate::render::backend::{ErasedSurface, ExternalHandle, PixelFormat, PresentMode, RenderBackend, RgbaImage, SurfaceSize};
use anyhow::{anyhow, Result};
use std::any::Any;

/// Null backend renderer that does not perform any rendering.
pub struct NullBackend;

impl NullBackend {
    /// Creates a new instance of the null backend.
    pub fn new() -> Result<Self> {
        Ok(Self)
    }
}

impl RenderBackend for NullBackend {
    fn name(&self) -> &'static str {
        "NullBackend"
    }

    fn create_surface(&self, size: SurfaceSize, _present: PresentMode) -> Result<Box<dyn ErasedSurface + Send>> {
        Ok(Box::new(NullSurface::new(size)?))
    }

    fn render(&self, _ctx: &mut BrowsingContext, surface: &mut dyn ErasedSurface) -> Result<()> {
        let s = surface
            .as_any_mut()
            .downcast_mut::<NullSurface>()
            .ok_or_else(|| anyhow!("NullBackend used with non-Null surface"))?;

        s.frame_id = s.frame_id.wrapping_add(1);
        Ok(())
    }

    fn snapshot(&self, surface: &mut dyn ErasedSurface, _max_dim: u32) -> Result<RgbaImage> {
        let s = surface
            .as_any_mut()
            .downcast_mut::<NullSurface>()
            .ok_or_else(|| anyhow!("NullBackend used with non-Null surface"))?;

        let pixels = vec![0u8; (s.size.width * s.size.height * 8) as usize];
        Ok(RgbaImage::from_raw(
            pixels,
            s.size.width,
            s.size.height,
            s.size.width * 4,
            PixelFormat::Rgba8,
        ))
    }

    fn external_handle(&self, surface: &mut dyn ErasedSurface) -> Result<ExternalHandle> {
        let s = surface.as_any_mut().downcast_mut::<NullSurface>().ok_or_else(|| anyhow!("NullBackend used with non-Null surface"))?;

        Ok(ExternalHandle::NullHandle {
            width: s.size.width,
            height: s.size.height,
            frame_id: s.frame_id,
        })
    }
}

/// A surface for the null backend that does not perform any actual rendering. It does however track the size and frame ID.
pub struct NullSurface {
    /// Size of the surface in pixels.
    pub size: SurfaceSize,
    /// Frame ID for the surface, used to track rendering frames.
    frame_id: u64,
}

impl NullSurface {
    pub fn new(size: SurfaceSize) -> Result<Self> {
        Ok(Self { size, frame_id: 0 })
    }
}

impl ErasedSurface for NullSurface {
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
