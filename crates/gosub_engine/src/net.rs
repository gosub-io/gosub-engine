//! # Gosub network stack
//!
//! Async network subsystem for HTTP/HTTPS fetching: a dedicated I/O runtime,
//! inflight-request coalescing (via the external `gosub-sonar` fetcher), body
//! streaming, and response routing into render-friendly outcomes.
//!
//! Fetching and body streaming run on a dedicated Tokio I/O thread
//! ([`spawn_io_thread`], [`submit_to_io`]); keep it for sockets, TLS, and disk
//! I/O - never block it with CPU-heavy work. What a response becomes is decided
//! after the I/O thread has replied, in the requesting tab's `tab-fetcher` task:
//! [`route_response_for`] classifies it, through [`decide_handling`], into a
//! [`HandlingDecision`] / [`RenderTarget`].
mod data_url;
mod decision;
mod decision_hub;
pub(crate) mod emitter;
pub use data_url::decode as decode_data_url;
pub mod brokered_loader;
pub mod events;
mod fetcher;
mod file_loader;
mod io_runtime;
pub mod orb;
/// The network stack running as a separate, sandboxed process.
#[cfg(feature = "process-isolation")]
pub mod process;
pub mod req_ref_tracker;
pub mod resource_loader;
mod router;
mod shared_body;
pub mod ssrf;
pub mod tab_identity;
pub mod types;
mod utils;

/// Make a handling decision for a routed response (e.g., render as document, hand to download manager).
pub use decision::decide_handling;
/// Common decision enums used across the network -> engine boundary.
pub use decision::types::{DecisionOutcome, HandlingDecision, RenderTarget, RequestDestination};
/// Token used to coordinate decisions across subsystems (e.g., to cancel or defer).
pub use decision_hub::DecisionToken;

/// Shared, back-pressure-aware streamed body used by fetcher and consumers.
pub use shared_body::SharedBody;

/// Spawn the dedicated Tokio I/O thread for all network work.
pub use io_runtime::spawn_io_thread;

/// Submit a closure/future to the I/O runtime, keeping network work off UI/main threads.
pub use io_runtime::submit_to_io;

/// Handle to the I/O runtime; cloneable and sendable across threads.
pub use io_runtime::IoHandle;

/// Configuration for the fetcher (timeouts, size limits, user agent, etc.).
pub use fetcher::FetcherConfig;

/// Build a [`FetcherConfig`] from the engine's settings store.
pub use fetcher::fetcher_config_from;

/// Compat-shaped `User-Agent` for this engine build, with an optional embedder product token.
pub use fetcher::default_user_agent;

/// Fully buffer a stream into bytes (tests, small assets, diagnostics).
pub use utils::stream_to_bytes;

/// Route a raw fetch result into a higher-level outcome the engine understands.
pub use router::route_response_for;

/// The routed outcome (MIME, sniffed type, charset, next steps).
pub use router::RoutedOutcome;
