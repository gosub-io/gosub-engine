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
        viewport_width: f64,
        viewport_height: f64,
    },
    /// Exit cleanly.
    Shutdown,
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
/// length from the wire).
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

/// What a forked renderer sends its parent over their private pair when asked
/// to render: the summary plus tile headers, followed by one sealed memfd per
/// header, in order.
#[derive(Debug, Serialize, Deserialize)]
pub struct RenderedPage {
    pub summary: PageSummary,
    pub tiles: Vec<TileHeader>,
}
