//! The cookie vault: the engine's cookie jars in a process of their own, so a
//! bug in the broker (which deserializes frames from every other child) is not
//! a bug with the session tokens in reach. See [`child`] for the model and
//! [`client`] for the broker's side; `security.cookie_vault` turns it on.

pub mod child;
pub mod client;
pub mod protocol;
