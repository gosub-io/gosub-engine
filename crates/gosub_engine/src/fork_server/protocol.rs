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
    PageRendered(PageSummary),
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
/// painted it. Numbers rather than pixels: enough for the broker to assert
/// the pipeline really ran (a dead font system collapses heights to zero; a
/// dead painter produces no commands), pending the shm tile transport.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageSummary {
    pub page_width: f64,
    pub page_height: f64,
    pub layer_count: u64,
    pub painted_tiles: u64,
    pub paint_commands: u64,
}
