//! The fork server: a warmed, sandboxed process renderers are forked from.
//!
//! Spawn is cheap (~3.7 ms measured); font warm-up is not, so it is paid once
//! here and forked renderers inherit the warmed font system copy-on-write.
//! The font system's [`Confinement`](gosub_interface::font_system::Confinement)
//! answer picks both this process's sandbox and its children's.

pub mod child;
pub mod client;
pub mod loader;
pub mod protocol;
pub mod renderer;
