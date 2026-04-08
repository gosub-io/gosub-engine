/// Default backend that doesn't render or return anything.
pub mod null;

/// Backend using the Cairo graphics library.
#[cfg(feature = "backend_cairo")]
pub mod cairo;

/// Backend using the Vello graphics library.
#[cfg(feature = "backend_vello")]
pub mod vello;

/// Backend using the Skia graphics library.
#[cfg(feature = "backend_skia")]
pub mod skia;
