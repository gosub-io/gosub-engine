//! Inter-process transport for the gosub engine: the byte channels between a
//! broker and its children, length-framed messaging over them, and the
//! shared-memory paths that keep bulk data (rendered tiles, response bodies)
//! out of those frames.
//!
//! Imported from the process-isolation proof of concept (gosub-engine#1080).
//! This crate is deliberately **protocol-free**: it carries any `serde` type
//! and knows nothing about renderers, fetches or tabs. Each boundary the engine
//! splits defines its own message types on top.
//!
//! The central abstraction is [`Endpoint`], which carries identical frames over
//! either a real cross-process channel or an in-process queue. Components are
//! written once against `Endpoint` and run unchanged as a child process or as a
//! thread — so a single-process build stays a supported configuration (wasm,
//! platforms without process isolation) rather than a second code path.
//!
//! ```text
//! endpoint  frames + fd passing        — any serde message type
//! channel   the byte transport seam    — unix socketpair / windows pipe pair
//! shm       sealed-memfd tiles         — Linux, zero-copy pixel hand-off
//! ring      shared-memory body stream  — Linux, bounded streaming
//! ```
//!
//! Modules are `pub` so untrusted-input parsers — notably [`endpoint::recv_msg`],
//! which decodes whatever a compromised child sends — can be driven directly by
//! tests and fuzzers.

// The transport seam: the only place a `target_os` cfg for the byte channel
// lives. Cross-process only — in-process links need no transport.
#[cfg(feature = "multi-process")]
pub mod channel;
pub mod endpoint;
// Shared memory rides on `SCM_RIGHTS` fd passing, so it follows the same
// Linux gate as the fd-passing half of `endpoint`.
#[cfg(all(feature = "multi-process", target_os = "linux"))]
pub mod ring;
#[cfg(all(feature = "multi-process", target_os = "linux"))]
pub mod shm;

pub use endpoint::{local_pair, Endpoint, EndpointRx, EndpointTx, MAX_FRAME_LEN};
