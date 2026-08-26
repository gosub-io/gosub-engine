//! # Gosub network stack
//!
//! A small, async-aware network subsystem that powers HTTP/HTTPS fetching for the
//! Gosub engine. It wraps a dedicated I/O runtime, inflight-request coalescing,
//! body streaming, and response routing into render-friendly outcomes.
//!
//! ## What this module provides
//! - A **dedicated Tokio I/O thread** for network work isolation (crate-internal:
//!   `spawn_io_thread`, `IoHandle`, `submit_to_io`).
//! - A **fetcher** (from the external `gosub-sonar` crate) with priority scheduling and
//!   inflight de-duplication to avoid duplicate downloads ([`FetcherConfig`]).
//! - A **shared, back-pressure-aware body** abstraction for streamed responses
//!   ([`SharedBody`]).
//! - A **router** that classifies responses and decides how the engine should handle them
//!   (crate-internal: `route_response_for`, `RoutedOutcome`; [`decide_handling`]).
//! - **Typed events** emitted during fetch & routing phases ([`events`]).
//!
//! Most of this is engine plumbing. What an embedder actually touches is [`types`]
//! (carried by navigation and resource events).
//!
//! ## Threading model (high level)
//! ```text
//! +------------------+             +--------------------+
//! | UI/Main threads  |  TabCmds    |   I/O thread       |
//! | (engine/tabs)    |-----------> | (Tokio runtime)    |
//! +------------------+             +--------------------+
//!        |                                   |
//!        | submit_to_io(...)                 | performs fetches, streams bodies
//!        v                                   v
//!  route_response_for(...)          SharedBody (cloneable readers)
//!        |
//!        v
//!  decide_handling(...)  ->  Decision + RenderTarget
//! ```
//!
//! ## Typical flow
//! 1. A tab (or engine) requests a URL; the work is **submitted** to the I/O thread with
//!    `submit_to_io` using the engine's `IoHandle`.
//! 2. The **fetcher** coalesces identical in-flight requests internally,
//!    producing either a **buffered** or **streamed** body (via [`SharedBody`]).
//! 3. The result is **routed** by `route_response_for` into a `RoutedOutcome` that carries type
//!    and metadata for downstream handling.
//! 4. The engine calls [`decide_handling`] to turn that into a concrete
//!    [`HandlingDecision`] / [`RenderTarget`] and proceeds accordingly.
//!
//! ## Notes & conventions
//! - **Never block** the I/O thread with CPU-heavy work; keep it for sockets, TLS, and disk I/O.
//! - Prefer **streaming** (`SharedBody`) for large responses; use **buffered** only when you need
//!   random access or small payloads.
//! - Emit and listen to [`events`] to keep UI and diagnostics reactive.
//!
//! ## Modules
//! The submodules below are internal implementation details unless re-exported. Public
//! items are documented via the re-exports that follow.
//!
mod decision;
mod emitter;
pub mod events;
mod fetcher;
mod file_loader;
mod io_runtime;
pub mod req_ref_tracker;
mod router;
mod shared_body;
pub mod types;
mod utils;

/// Make a **handling decision** for a routed response (e.g., render as document, hand to download manager).
pub use decision::decide_handling;
/// Common decision enums used across the network -> engine boundary.
pub use decision::types::{BlockReason, DecisionOutcome, HandlingDecision, RenderTarget, RequestDestination};
/// Shared, back-pressure-aware **streamed body** used by fetcher and consumers.
pub use shared_body::SharedBody;

/// Spawn the dedicated **Tokio I/O thread** for all network work.
///
/// Returns an [`IoHandle`] you can clone and pass around.
pub(crate) use io_runtime::spawn_io_thread;

/// Submit a closure/future to the I/O runtime for execution.
///
/// Keeps network work off UI/main threads.
pub(crate) use io_runtime::submit_to_io;

/// Handle to the I/O runtime; cloneable and sendable across threads.
pub(crate) use io_runtime::IoHandle;

/// Configuration for the fetcher (timeouts, size limits, user agent, etc.).
pub use fetcher::FetcherConfig;

/// Build a [`FetcherConfig`] from the engine's settings store.
pub use fetcher::fetcher_config_from;

/// Compat-shaped `User-Agent` for this engine build, with an optional embedder product token.
pub use fetcher::default_user_agent;

/// Utility to **fully buffer a stream** into bytes (tests, small assets, diagnostics).
pub(crate) use utils::stream_to_bytes;

/// Route a raw fetch result into a higher-level outcome the engine understands.
pub(crate) use router::route_response_for;

/// The routed outcome (MIME, sniffed type, charset, next steps).
pub(crate) use router::RoutedOutcome;
