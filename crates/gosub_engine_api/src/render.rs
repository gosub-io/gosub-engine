pub mod backend;

/// Rendering system: backends, surfaces, and display lists.
///
/// The `render` module provides the abstraction layer between the Gosub engine
/// and concrete GPU/CPU drawing libraries. It exposes:
///
/// - **Backends** in `render::backends` (e.g., `cairo`, `vello`, `null`) that
///   implement the engine’s rendering contract.
/// - A **display list** (re-exported from `render_list`) describing what to
///   paint in a backend-neutral way.
/// - A [`Viewport`] that defines the visible region and size of a tab or canvas.
/// - A **compositor interface** (see `DefaultCompositor`) used by host
///   applications to present frames produced by the backend.
///
/// ## Architecture at a glance
///
/// 1. The engine builds a backend-agnostic **display list** for a given tab.
/// 2. A **backend** consumes the display list and produces a **ExternalHandle**
///    (GPU surface texture, CPU image, etc.).
/// 3. The **compositor** receives the external handle and integrates it into the
///    host UI (e.g., a GTK widget, a winit window, or an offscreen surface).
///
/// The split lets you embed Gosub in different hosts and swap rendering
/// technologies without touching core engine code.
///
/// ## Backends
///
/// Backends live under `render::backends` and are selected by feature flags:
///
/// - `backend_cairo` -> CPU raster via Cairo (`render::backends::cairo`)
/// - `backend_vello` -> GPU (wgpu) via Vello (`render::backends::vello`)
/// - always available: `render::backends::null` (no-op, useful for tests)
///
/// Because these modules are feature-gated, this documentation refers to them
/// using inline code (not links) to avoid broken intra-doc links when a feature
/// is disabled.
///
/// ## Viewport
///
/// A [`Viewport`] specifies `(x, y, width, height)` in pixels to define the
/// visible area that a backend should paint. Hosts typically update the
/// viewport on resize or scrolling.
///
/// ```no_run
/// use gosub_engine_api::render::Viewport;
///
/// // 800×600 viewport at origin
/// let mut vp = Viewport::new(0, 0, 800, 600);
/// vp.translate(0, 120);     // e.g. after scrolling
/// vp.resize(1280, 720);     // e.g. after a window resize
/// ```
///
/// ## Display list (render list)
///
/// The engine emits a display list (re-exported from this module) containing
/// shapes, images, text runs, and state. Backends convert this list into the
/// target API’s primitives. Hosts don’t usually touch the display list
/// directly; they drive tabs and submit frames to the compositor.
///
/// ## Compositing
///
/// The compositor is implemented by the host application. The engine will call
/// into it (e.g., via `DefaultCompositor`) to hand over a ExternalHandle that the
/// host can present in its UI. This keeps the engine independent from any
/// specific windowing toolkit.
///
/// ## Typical flow
///
/// ```no_run
/// # #[cfg(feature = "backend_vello")]
/// # fn demo() -> anyhow::Result<()> {
/// use gosub_engine_api::render::{Viewport, /* DefaultCompositor, backends */};
/// // 1) Host sets up a compositor and a backend (e.g. Vello).
/// //    Exact types depend on the selected backend feature.
/// // let mut compositor = DefaultCompositor::new(host_handle);
/// // let mut backend = backends::vello::Backend::new(&wgpu_device, &wgpu_queue)?;
///
/// // 2) Engine produces/updates a display list for a tab.
/// // let display_list = engine.build_display_list(tab_id);
///
/// // 3) Backend renders the display list into a frame.
/// let vp = Viewport::new(0, 0, 1280, 720);
/// // let frame = backend.render(&display_list, &vp)?;
///
/// // 4) Host composits the frame into its UI.
/// // compositor.submit_frame(tab_id, frame);
///
/// # Ok(()) }
/// ```
///
/// ## Feature flags
///
/// - `backend_cairo` – enable Cairo CPU backend
/// - `backend_vello` – enable Vello (wgpu) GPU backend
///
/// Enable one (or both) in `Cargo.toml` depending on your target environment.
///
/// ## Notes
/// - **Threading:** GPU backends may require creation and use on specific
///   threads depending on the windowing layer. Create surfaces/queues where the
///   windowing API expects them.
/// - **Presentation:** The engine doesn’t present frames directly; the host’s
///   compositor owns that responsibility.
pub mod backends {
    /// Cairo rendering backend
    #[cfg(feature = "backend_cairo")]
    pub mod cairo;
    /// Default backend that doesn't render or return anything.
    pub mod null;
    /// Vello rendering backend
    #[cfg(feature = "backend_vello")]
    pub mod vello;
}

mod render_list;
pub use render_list::*;

mod viewport;
pub use viewport::DevicePixelRatio;
pub use viewport::Viewport;

mod compositor;
mod compositor_router;

pub use compositor::DefaultCompositor;
pub use compositor_router::CompositorRouter;
