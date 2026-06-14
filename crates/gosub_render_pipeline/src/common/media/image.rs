//! `Image` is the in-store representation of a decoded raster image. It is the normalized
//! [`DecodedImage`](super::DecodedImage) produced by the decoder registry, exposing
//! `width()`/`height()`/`as_raw()` for renderers and layout.
pub use super::DecodedImage as Image;
