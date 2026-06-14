//! A runtime-selectable render backend.
//!
//! [`DynamicRenderBackend`] bundles several concrete render backends (Cairo, Skia, Vello) behind
//! a single [`RenderBackend`] and delegates every call to the one currently selected. This is the
//! *only* place in the workspace that knows about the concrete backends together — the render
//! pipeline and the engine stay fully renderer-agnostic and only ever see `dyn RenderBackend`.
//!
//! A host enables the backends it can build on its platform via crate features (`cairo`, `skia`,
//! `vello`) and registers them through the builder. Selection is by [`RenderBackendKind`] and can
//! change at runtime via [`DynamicRenderBackend::set_active`].

use std::collections::HashMap;
use std::sync::Arc;

use gosub_render_pipeline::render::backend::{
    ErasedSurface, ExternalHandle, PresentMode, RenderBackend, RgbaImage, SurfaceSize,
};
use gosub_render_pipeline::render::backends::null::NullBackend;
use gosub_render_pipeline::render::render_context::RenderContext;
use parking_lot::RwLock;

/// Identifies a concrete render backend. Defined here, never in the pipeline, so the pipeline
/// remains agnostic of which backends exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RenderBackendKind {
    /// The null backend (renders nothing).
    Null,
    /// Cairo (CPU, GTK/Pango).
    Cairo,
    /// Skia (CPU raster).
    Skia,
    /// Vello (wgpu GPU).
    Vello,
}

type BoxedBackend = Arc<dyn RenderBackend + Send + Sync>;

/// A [`RenderBackend`] that holds several backends at once and delegates to the active one.
///
/// Build it with [`DynamicRenderBackend::builder`], then hand it to the engine as
/// `Arc<dyn RenderBackend>`. Keep a clone of the `Arc<DynamicRenderBackend>` if you want to
/// switch backends at runtime with [`set_active`](Self::set_active).
pub struct DynamicRenderBackend {
    backends: HashMap<RenderBackendKind, BoxedBackend>,
    null: BoxedBackend,
    active: RwLock<RenderBackendKind>,
}

impl DynamicRenderBackend {
    /// Starts building a dynamic backend.
    pub fn builder() -> DynamicRenderBackendBuilder {
        DynamicRenderBackendBuilder::default()
    }

    /// Selects the active backend by kind. Returns `false` (keeping the current selection) if no
    /// backend of that kind was registered.
    pub fn set_active(&self, kind: RenderBackendKind) -> bool {
        if kind == RenderBackendKind::Null || self.backends.contains_key(&kind) {
            *self.active.write() = kind;
            true
        } else {
            false
        }
    }

    /// The kind of the currently active backend.
    pub fn active_kind(&self) -> RenderBackendKind {
        *self.active.read()
    }

    #[inline]
    fn active_backend(&self) -> BoxedBackend {
        let kind = *self.active.read();
        self.backends
            .get(&kind)
            .cloned()
            .unwrap_or_else(|| Arc::clone(&self.null))
    }
}

impl RenderBackend for DynamicRenderBackend {
    fn name(&self) -> &'static str {
        self.active_backend().name()
    }

    fn create_surface(&self, size: SurfaceSize, present: PresentMode) -> anyhow::Result<Box<dyn ErasedSurface + Send>> {
        self.active_backend().create_surface(size, present)
    }

    fn render(&self, context: &mut dyn RenderContext, surface: &mut dyn ErasedSurface) -> anyhow::Result<()> {
        self.active_backend().render(context, surface)
    }

    fn snapshot(&self, surface: &mut dyn ErasedSurface, max_dim: u32) -> anyhow::Result<RgbaImage> {
        self.active_backend().snapshot(surface, max_dim)
    }

    fn external_handle(&self, surface: &mut dyn ErasedSurface) -> anyhow::Result<ExternalHandle> {
        self.active_backend().external_handle(surface)
    }

    fn wgpu_resources(&self) -> Option<Arc<dyn std::any::Any + Send + Sync>> {
        self.active_backend().wgpu_resources()
    }

    fn create_rasterizer(&self) -> Box<dyn gosub_render_pipeline::rasterizer::Rasterable + Send + Sync> {
        self.active_backend().create_rasterizer()
    }

    fn raster_strategy(&self) -> gosub_render_pipeline::rasterizer::RasterStrategy {
        self.active_backend().raster_strategy()
    }

    fn device_pixel_ratio(&self) -> u32 {
        self.active_backend().device_pixel_ratio()
    }
}

/// Builder for [`DynamicRenderBackend`].
///
/// Register the backends the host can construct, optionally pick the initial active one with
/// [`active`](Self::active) (otherwise the first registered backend is active), then
/// [`build`](Self::build).
#[derive(Default)]
pub struct DynamicRenderBackendBuilder {
    backends: HashMap<RenderBackendKind, BoxedBackend>,
    active: Option<RenderBackendKind>,
    order: Vec<RenderBackendKind>,
}

impl DynamicRenderBackendBuilder {
    /// Registers an already-constructed backend under `kind`. Escape hatch for backends not
    /// covered by the `with_*` helpers (or to override one).
    pub fn register(mut self, kind: RenderBackendKind, backend: BoxedBackend) -> Self {
        if !self.backends.contains_key(&kind) {
            self.order.push(kind);
        }
        self.backends.insert(kind, backend);
        self
    }

    /// Registers a Cairo backend.
    #[cfg(feature = "cairo")]
    pub fn with_cairo(self) -> Self {
        self.register(
            RenderBackendKind::Cairo,
            Arc::new(gosub_renderer_cairo::CairoBackend::new()),
        )
    }

    /// Registers a Skia (CPU) backend.
    #[cfg(feature = "skia")]
    pub fn with_skia(self) -> Self {
        self.register(
            RenderBackendKind::Skia,
            Arc::new(gosub_renderer_skia::SkiaBackend::new()),
        )
    }

    /// Constructs and registers a Vello backend from the host's wgpu context provider.
    #[cfg(feature = "vello")]
    pub fn with_vello<C>(self, context: Arc<C>) -> anyhow::Result<Self>
    where
        C: gosub_renderer_vello::WgpuContextProvider + Send + Sync + 'static,
    {
        let backend = gosub_renderer_vello::VelloBackend::new(context)?;
        Ok(self.register(RenderBackendKind::Vello, Arc::new(backend)))
    }

    /// Sets the initially active backend kind (defaults to the first registered backend).
    pub fn active(mut self, kind: RenderBackendKind) -> Self {
        self.active = Some(kind);
        self
    }

    /// Finalizes the dynamic backend.
    pub fn build(self) -> DynamicRenderBackend {
        let active = self
            .active
            .or_else(|| self.order.first().copied())
            .unwrap_or(RenderBackendKind::Null);
        DynamicRenderBackend {
            backends: self.backends,
            null: Arc::new(NullBackend::new()),
            active: RwLock::new(active),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_registered_and_rejects_unregistered() {
        let null: BoxedBackend = Arc::new(NullBackend::new());
        // Register two backends under distinct kinds (using null instances as stand-ins).
        let dynamic = DynamicRenderBackend::builder()
            .register(RenderBackendKind::Cairo, Arc::clone(&null))
            .register(RenderBackendKind::Vello, Arc::clone(&null))
            .build();

        // First registered becomes active.
        assert_eq!(dynamic.active_kind(), RenderBackendKind::Cairo);
        // Switch to a registered kind.
        assert!(dynamic.set_active(RenderBackendKind::Vello));
        assert_eq!(dynamic.active_kind(), RenderBackendKind::Vello);
        // Unregistered kind is rejected; selection unchanged.
        assert!(!dynamic.set_active(RenderBackendKind::Skia));
        assert_eq!(dynamic.active_kind(), RenderBackendKind::Vello);
        // Null is always selectable.
        assert!(dynamic.set_active(RenderBackendKind::Null));
    }
}
