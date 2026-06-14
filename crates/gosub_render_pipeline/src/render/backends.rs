/// Default backend that doesn't render or return anything.
pub mod null;

/// Backend using the Vello graphics library.
#[cfg(feature = "backend_vello")]
pub mod vello;
