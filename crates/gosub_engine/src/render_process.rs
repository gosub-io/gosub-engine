//! The exec-fresh renderer: one page render in its own throwaway process.
//!
//! How `FontPathsReadable` configurations get renderer isolation. Their font
//! stacks read font files while operating, so fork-server font warm-up buys
//! nothing, and such a stack may not even be constructible inside the fork
//! server (GLib insists on a worker thread; the fork server's PID-namespace
//! unshare forbids thread creation). These renderers are instead exec'd fresh
//! per render (~3.7 ms measured), confined with the font-readable profile,
//! and exit after one page.

pub mod child;
pub mod client;
