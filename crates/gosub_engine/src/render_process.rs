//! The exec-fresh renderer: one page render in its own throwaway process.
//!
//! This is how `FontPathsReadable` configurations get renderer isolation.
//! Their font stacks read font files *while operating*, so the fork server's
//! whole reason to exist — warm fonts once, inherit them copy-on-write — buys
//! them nothing; worse, such a stack may not even be constructible inside the
//! fork server (GLib insists on a worker thread, and the fork server's
//! PID-namespace unshare forbids thread creation). So these renderers are
//! spawned the way the decoder is: exec'd fresh per render (~3.7 ms, measured
//! long ago as cheap enough), confined with the font-readable profile, gone
//! when the page is rendered — the "one image, then gone" philosophy applied
//! to pages.

pub mod child;
pub mod client;
