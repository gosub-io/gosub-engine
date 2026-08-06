//! The broker↔fork-server wire vocabulary.

use serde::{Deserialize, Serialize};

/// The confinement answer, as it crosses the process boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConfinementTier {
    /// The font system front-loaded everything; renderers get the strictest
    /// sandbox (no file access at all).
    Full,
    /// The font system reads font files while operating; renderers get
    /// read-only font paths plus a private writable scratch.
    FontPathsReadable,
    /// The font system cannot run isolated; the fork server refuses to fork
    /// and the engine must render single-process.
    Unsupported(String),
}

impl From<&gosub_interface::font_system::Confinement> for ConfinementTier {
    fn from(answer: &gosub_interface::font_system::Confinement) -> Self {
        use gosub_interface::font_system::Confinement;
        match answer {
            Confinement::Full => ConfinementTier::Full,
            Confinement::FontPathsReadable => ConfinementTier::FontPathsReadable,
            Confinement::Unsupported(reason) => ConfinementTier::Unsupported(reason.clone()),
        }
    }
}

/// Broker → fork server.
#[derive(Debug, Serialize, Deserialize)]
pub enum ToForkServer {
    /// Liveness check.
    Ping,
    /// Fork a renderer, confine it to the announced tier, shape text with the
    /// inherited (copy-on-write) font system, and report the measured box.
    ForkProof,
    /// Fork a renderer and run the render pipeline in it: parse `html`, style,
    /// lay out against the viewport, layer, tile, and paint — single-threaded,
    /// under the announced tier's sandbox, measuring and shaping through the
    /// inherited font system. Replies with [`FromForkServer::PageRendered`].
    RenderPage {
        html: String,
        /// The page's URL — the base against which the renderer resolves
        /// relative subresource URLs (stylesheets, images, fonts).
        url: String,
        viewport_width: f64,
        viewport_height: f64,
    },
    /// Exit cleanly.
    Shutdown,
    /// The broker's answer to [`FromForkServer::NeedResource`], relayed on to
    /// the renderer that is blocked waiting for it. Only ever sent while a
    /// [`RenderPage`](ToForkServer::RenderPage) exchange is in flight.
    Resource(ResourceReply),
}

/// Fork server → broker.
#[derive(Debug, Serialize, Deserialize)]
pub enum FromForkServer {
    /// Sent once, after the font system answered and the fork server confined
    /// itself accordingly: it is warmed, sandboxed, and ready to fork.
    Ready { tier: ConfinementTier },
    /// Liveness reply.
    Pong,
    /// A forked renderer shaped text under its tier sandbox and measured this.
    Proof { width: f32, height: f32 },
    /// A forked renderer ran the pipeline over a page and measured this.
    ///
    /// After this message, one sealed-memfd file descriptor follows on the
    /// link **per entry in `tiles`, in order** — the pixels themselves never
    /// travel in-band (see `gosub_ipc::shm`). An empty `tiles` means the
    /// configuration has no forked rasterizer and stage 6 was skipped.
    PageRendered {
        summary: PageSummary,
        tiles: Vec<TileHeader>,
    },
    /// The request could not be served; the string says why (e.g. forking is
    /// refused under `Unsupported`, or the forked child died).
    Refused(String),
    /// A renderer needs a subresource it has no capability to fetch — the
    /// brokered-load inversion, mirroring cookies: the renderer names what it
    /// wants, the broker performs the fetch where identity and cookies live,
    /// and only bytes come back. Sent mid-[`RenderPage`](ToForkServer::RenderPage);
    /// the broker answers with [`ToForkServer::Resource`] before anything else.
    NeedResource { url: String },
}

/// What a forked renderer sends its parent over their private pair before
/// exiting. Internal to the fork-server process family, but it crosses a
/// process boundary (fork), so it is wire vocabulary all the same.
#[derive(Debug, Serialize, Deserialize)]
pub struct ProofReply {
    pub width: f32,
    pub height: f32,
}

/// What a page came to, measured by the forked renderer that laid it out and
/// painted it. Numbers rather than pixels — the pixels travel separately, as
/// sealed memfds — but enough on their own for the broker to assert the
/// pipeline really ran (a dead font system collapses heights to zero; a dead
/// painter produces no commands).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageSummary {
    pub page_width: f64,
    pub page_height: f64,
    pub layer_count: u64,
    pub painted_tiles: u64,
    pub paint_commands: u64,
}

/// Everything about one rasterized tile except its pixels, which follow as a
/// sealed memfd (see `gosub_ipc::shm` — the consumer derives the byte count
/// from these dimensions and validates the fd against them, never trusting a
/// length from the wire). Carries what the compositor's `CachedTile` needs,
/// so a mapped tile converts without consulting the renderer again.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TileHeader {
    /// Position of this tile on the page, in CSS pixels.
    pub page_x: f64,
    pub page_y: f64,
    /// Owning layer: `(0,0)` can exist in both a base layer and a sticky one.
    pub layer_id: u64,
    pub width: u32,
    pub height: u32,
    pub format: TileWireFormat,
    /// Group opacity of the tile's layer, applied by the compositor.
    pub opacity: f32,
    /// How the tile's layer responds to scroll.
    pub anchor: TileWireAnchor,
}

/// The in-memory byte order of a shipped tile — the wire mirror of the
/// interface crate's `PixelFormat` (which carries no serde).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TileWireFormat {
    /// Little-endian premultiplied ARGB32 (`[B, G, R, A]`): Cairo, Skia.
    PreMulArgb32,
    /// Premultiplied RGBA8: Vello.
    Rgba8,
}

impl From<gosub_interface::render::backend::PixelFormat> for TileWireFormat {
    fn from(format: gosub_interface::render::backend::PixelFormat) -> Self {
        use gosub_interface::render::backend::PixelFormat;
        match format {
            PixelFormat::PreMulArgb32 => TileWireFormat::PreMulArgb32,
            PixelFormat::Rgba8 => TileWireFormat::Rgba8,
        }
    }
}

impl From<TileWireFormat> for gosub_interface::render::backend::PixelFormat {
    fn from(format: TileWireFormat) -> Self {
        use gosub_interface::render::backend::PixelFormat;
        match format {
            TileWireFormat::PreMulArgb32 => PixelFormat::PreMulArgb32,
            TileWireFormat::Rgba8 => PixelFormat::Rgba8,
        }
    }
}

/// Wire mirror of the interface crate's `TileAnchor` (no serde there).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum TileWireAnchor {
    Scroll,
    Fixed,
    Sticky(StickyWire),
}

/// Wire mirror of `StickyConstraint`: plain page-space geometry.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct StickyWire {
    pub inset_top: Option<f64>,
    pub inset_left: Option<f64>,
    pub natural_x: f64,
    pub natural_y: f64,
    pub natural_w: f64,
    pub natural_h: f64,
    pub cage_x: f64,
    pub cage_y: f64,
    pub cage_w: f64,
    pub cage_h: f64,
}

impl From<gosub_interface::render::backend::TileAnchor> for TileWireAnchor {
    fn from(anchor: gosub_interface::render::backend::TileAnchor) -> Self {
        use gosub_interface::render::backend::TileAnchor;
        match anchor {
            TileAnchor::Scroll => TileWireAnchor::Scroll,
            TileAnchor::Fixed => TileWireAnchor::Fixed,
            TileAnchor::Sticky(s) => TileWireAnchor::Sticky(StickyWire {
                inset_top: s.inset_top,
                inset_left: s.inset_left,
                natural_x: s.natural_x,
                natural_y: s.natural_y,
                natural_w: s.natural_w,
                natural_h: s.natural_h,
                cage_x: s.cage_x,
                cage_y: s.cage_y,
                cage_w: s.cage_w,
                cage_h: s.cage_h,
            }),
        }
    }
}

impl From<TileWireAnchor> for gosub_interface::render::backend::TileAnchor {
    fn from(anchor: TileWireAnchor) -> Self {
        use gosub_interface::render::backend::{StickyConstraint, TileAnchor};
        match anchor {
            TileWireAnchor::Scroll => TileAnchor::Scroll,
            TileWireAnchor::Fixed => TileAnchor::Fixed,
            TileWireAnchor::Sticky(s) => TileAnchor::Sticky(StickyConstraint {
                inset_top: s.inset_top,
                inset_left: s.inset_left,
                natural_x: s.natural_x,
                natural_y: s.natural_y,
                natural_w: s.natural_w,
                natural_h: s.natural_h,
                cage_x: s.cage_x,
                cage_y: s.cage_y,
                cage_w: s.cage_w,
                cage_h: s.cage_h,
            }),
        }
    }
}

/// What a forked renderer sends its parent over their private pair when asked
/// to render: the summary plus tile headers, followed by one sealed memfd per
/// header, in order.
#[derive(Debug, Serialize, Deserialize)]
pub struct RenderedPage {
    pub summary: PageSummary,
    pub tiles: Vec<TileHeader>,
}

/// Everything a renderer can say to its parent over their private pair.
#[derive(Debug, Serialize, Deserialize)]
pub enum FromRenderer {
    /// Mid-render: fetch this for me. The parent relays it to the broker and
    /// sends the [`ResourceReply`] back; the renderer is blocked until then.
    NeedResource { url: String },
    /// The final message: the page, with fds to follow.
    Rendered(RenderedPage),
}

/// A fetched subresource (or its failure), as it travels broker → fork server
/// → renderer. Mirrors `gosub_interface::resource_loader::LoadedResource`,
/// which carries no serde.
#[derive(Debug, Serialize, Deserialize)]
pub enum ResourceReply {
    Ok {
        status: u16,
        content_type: Option<String>,
        body: Vec<u8>,
    },
    Failed(String),
}
