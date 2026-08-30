//! `localStorage` in a sandboxed process; embedders opt in with
//! [`client::ServiceLocalStore`].

pub mod child;
pub mod client;
pub mod protocol;
