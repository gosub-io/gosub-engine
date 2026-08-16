//! The duplex byte channel an engine↔component IPC link runs over.

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::{Channel, Rx, Tx};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{Channel, Rx, Tx};
