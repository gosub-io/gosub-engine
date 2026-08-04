//! The fork server: a warmed, sandboxed process renderers are forked from.
//!
//! Phase 4 of process isolation starts here. Measurements set its shape: spawn
//! is already cheap (~3.7 ms), but font warm-up is only worth paying **once**
//! — so the fork server builds the embedder's configured font system, consumes
//! its [`Confinement`](gosub_interface::font_system::Confinement) answer to
//! pick both its own sandbox and its children's, and forks renderers that
//! inherit the warmed fonts copy-on-write.

pub mod child;
pub mod client;
pub mod protocol;
pub mod renderer;
