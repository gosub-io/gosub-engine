use crate::render::backend::{ErasedSurface, ExternalHandle, PresentMode, RenderBackend, RgbaImage, SurfaceSize};
use crate::render::render_context::RenderContext;
use crate::render::render_list::{Color, DisplayItem};
use anyhow::{anyhow, Result};
use parking_lot::Mutex;
use skia_safe::Color4f;
use std::any::Any;
use std::sync::Arc;

#[inline]
pub fn to_color4f(c: &Color) -> Color4f {
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
        Self {
            pending: Arc::new(Mutex::new(None)),
        }
    }
}

impl Default for SkiaGpuDirectFbBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderBackend for SkiaGpuDirectFbBackend {
    fn name(&self) -> &'static str {
        "skia-gpu-direct-fb"
    }

    fn create_surface(&self, size: SurfaceSize, present: PresentMode) -> Result<Box<dyn ErasedSurface + Send>> {
        Ok(Box::new(SkiaDirectFbSurface {
            size,
            frame_id: 0,
            present,
        }))
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
        Err(anyhow!(
            "SkiaGpuDirectFbBackend: snapshot not supported (frame is on GPU)"
        ))
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
