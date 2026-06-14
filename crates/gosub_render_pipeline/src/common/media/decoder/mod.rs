//! Pluggable media decoding.
//!
//! [`MediaStore`](crate::common::media::MediaStore) does not care whether a resource is PNG,
//! JPEG, GIF or SVG — it hands the raw bytes (and an optional MIME hint) to a
//! [`MediaDecoderRegistry`], which picks a [`MediaDecoder`] and produces a normalized
//! [`DecodedMedia`].
//!
//! Raster formats are normalized to [`PixelBuffer::Rgba8`]; SVG is kept as a retained
//! `usvg::Tree` ([`DecodedMedia::Vector`]) so it can be re-rasterized crisply at any size.

mod raster;
mod svg;

pub use raster::RasterDecoder;
pub use svg::SvgDecoder;

use std::fmt;

/// Pixel storage for a decoded raster image. Only 8-bit RGBA is supported today; the enum
/// leaves room for wider/greyscale buffers without churning the public surface.
#[derive(Clone, PartialEq, Eq)]
pub enum PixelBuffer {
    Rgba8(Vec<u8>),
}

/// A decoded raster image, normalized to a single internal representation.
#[derive(Clone, PartialEq, Eq)]
pub struct DecodedImage {
    width: u32,
    height: u32,
    pixels: PixelBuffer,
}

impl DecodedImage {
    /// Build an RGBA8 image. Returns [`ImageDecodeError::Decode`] when the buffer length does
    /// not match `width * height * 4`, rather than panicking.
    pub fn new_rgba8(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self, ImageDecodeError> {
        let expected = (width as usize) * (height as usize) * 4;
        if pixels.len() != expected {
            return Err(ImageDecodeError::Decode(format!(
                "RGBA8 buffer length {} does not match {}x{}x4={}",
                pixels.len(),
                width,
                height,
                expected
            )));
        }
        Ok(Self {
            width,
            height,
            pixels: PixelBuffer::Rgba8(pixels),
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn pixels(&self) -> &PixelBuffer {
        &self.pixels
    }

    /// Borrow the raw pixel bytes. For [`PixelBuffer::Rgba8`] this is tightly-packed RGBA.
    pub fn as_raw(&self) -> &[u8] {
        match &self.pixels {
            PixelBuffer::Rgba8(bytes) => bytes,
        }
    }
}

impl fmt::Debug for DecodedImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DecodedImage")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("bytes", &self.as_raw().len())
            .finish()
    }
}

/// What a [`MediaDecoder`] produces: either a normalized raster image or a retained vector
/// tree that downstream code re-rasterizes per render size.
pub enum DecodedMedia {
    Raster(DecodedImage),
    // Boxed: a `usvg::Tree` is far larger than `DecodedImage`, so boxing keeps the enum small.
    Vector(Box<resvg::usvg::Tree>),
}

impl fmt::Debug for DecodedMedia {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodedMedia::Raster(img) => f.debug_tuple("Raster").field(img).finish(),
            DecodedMedia::Vector(_) => f.write_str("Vector(usvg::Tree)"),
        }
    }
}

/// Failure modes when decoding media bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageDecodeError {
    /// No registered decoder claimed the MIME type or recognised the magic bytes.
    UnsupportedFormat,
    /// A decoder claimed the bytes but failed to decode them.
    Decode(String),
}

impl fmt::Display for ImageDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImageDecodeError::UnsupportedFormat => f.write_str("unsupported media format"),
            ImageDecodeError::Decode(msg) => write!(f, "failed to decode media: {msg}"),
        }
    }
}

impl std::error::Error for ImageDecodeError {}

/// A single format handler. Decoders are matched first by MIME hint, then by magic bytes.
pub trait MediaDecoder: Send + Sync {
    /// Short, stable identifier (used in logs).
    fn name(&self) -> &'static str;

    /// Whether this decoder handles the given MIME type. Implementations must compare
    /// case-insensitively and tolerate `;`-delimited parameters.
    fn supports_mime(&self, mime: &str) -> bool;

    /// Whether the leading bytes look like a format this decoder handles.
    fn supports_magic(&self, bytes: &[u8]) -> bool;

    /// Decode the bytes into normalized media.
    fn decode(&self, bytes: &[u8]) -> Result<DecodedMedia, ImageDecodeError>;
}

/// Ordered set of decoders. The MIME hint (e.g. an HTTP `Content-Type`) is treated as a hint
/// only — servers frequently send the wrong type, so a MIME-matched decoder that fails to
/// decode falls through to magic-byte sniffing before giving up.
pub struct MediaDecoderRegistry {
    decoders: Vec<Box<dyn MediaDecoder>>,
}

impl MediaDecoderRegistry {
    /// An empty registry. Use [`MediaDecoderRegistry::register`] to add decoders, or
    /// [`MediaDecoderRegistry::with_defaults`] for the built-in set.
    pub fn new() -> Self {
        Self { decoders: Vec::new() }
    }

    /// The built-in decoder set: every raster format the `image` crate is compiled with,
    /// plus SVG.
    pub fn with_defaults() -> Self {
        let mut registry = Self::new();
        registry.register(Box::new(SvgDecoder::new()));
        registry.register(Box::new(RasterDecoder));
        registry
    }

    pub fn register(&mut self, decoder: Box<dyn MediaDecoder>) {
        self.decoders.push(decoder);
    }

    /// Decode `bytes`, using `mime` as a hint. Tries MIME-matched decoders first, then falls
    /// back to magic-byte sniffing.
    pub fn decode(&self, mime: Option<&str>, bytes: &[u8]) -> Result<DecodedMedia, ImageDecodeError> {
        let mut last_err: Option<ImageDecodeError> = None;

        if let Some(mime) = mime {
            for decoder in &self.decoders {
                if decoder.supports_mime(mime) {
                    match decoder.decode(bytes) {
                        Ok(media) => return Ok(media),
                        Err(e) => {
                            log::debug!("decoder '{}' (mime '{}') failed: {}", decoder.name(), mime, e);
                            last_err = Some(e);
                        }
                    }
                }
            }
        }

        for decoder in &self.decoders {
            if decoder.supports_magic(bytes) {
                match decoder.decode(bytes) {
                    Ok(media) => return Ok(media),
                    Err(e) => {
                        log::debug!("decoder '{}' (magic) failed: {}", decoder.name(), e);
                        last_err = Some(e);
                    }
                }
            }
        }

        Err(last_err.unwrap_or(ImageDecodeError::UnsupportedFormat))
    }
}

impl Default for MediaDecoderRegistry {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
    use std::io::Cursor;

    /// Encode a small solid image into `format`, returning the raw bytes.
    fn encode(format: ImageFormat) -> Vec<u8> {
        let rgba = DynamicImage::ImageRgba8(RgbaImage::from_pixel(8, 4, Rgba([200, 100, 50, 255])));
        let mut buf = Cursor::new(Vec::new());
        match format {
            // JPEG has no alpha channel, so encode from an RGB view.
            ImageFormat::Jpeg => DynamicImage::ImageRgb8(rgba.to_rgb8())
                .write_to(&mut buf, format)
                .expect("encode jpeg"),
            _ => rgba.write_to(&mut buf, format).expect("encode image"),
        }
        buf.into_inner()
    }

    const SVG: &[u8] = br#"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg" width="10" height="6"><rect width="10" height="6" fill="red"/></svg>"#;

    fn expect_raster(media: DecodedMedia) -> DecodedImage {
        match media {
            DecodedMedia::Raster(img) => img,
            other => panic!("expected raster, got {other:?}"),
        }
    }

    #[test]
    fn decodes_raster_formats_via_magic() {
        let registry = MediaDecoderRegistry::with_defaults();
        for format in [ImageFormat::Png, ImageFormat::Jpeg, ImageFormat::Gif] {
            let bytes = encode(format);
            let media = registry
                .decode(None, &bytes)
                .unwrap_or_else(|e| panic!("{format:?}: {e}"));
            let img = expect_raster(media);
            assert_eq!((img.width(), img.height()), (8, 4), "{format:?} dimensions");
            assert_eq!(img.as_raw().len(), 8 * 4 * 4, "{format:?} rgba length");
        }
    }

    #[test]
    fn decodes_raster_via_mime_hint() {
        let registry = MediaDecoderRegistry::with_defaults();
        let bytes = encode(ImageFormat::Png);
        let media = registry.decode(Some("image/png"), &bytes).expect("png via mime");
        assert_eq!(expect_raster(media).width(), 8);
    }

    #[test]
    fn decodes_svg_to_retained_tree() {
        let registry = MediaDecoderRegistry::with_defaults();
        // via mime
        assert!(matches!(
            registry.decode(Some("image/svg+xml"), SVG),
            Ok(DecodedMedia::Vector(_))
        ));
        // via magic only
        assert!(matches!(registry.decode(None, SVG), Ok(DecodedMedia::Vector(_))));
    }

    #[test]
    fn mime_first_then_falls_back_to_magic_on_mismatch() {
        // Server lies: claims PNG but the bytes are SVG. RasterDecoder claims the MIME,
        // fails to decode, and we fall through to the SVG magic sniff.
        let registry = MediaDecoderRegistry::with_defaults();
        let media = registry.decode(Some("image/png"), SVG).expect("fallback to svg");
        assert!(matches!(media, DecodedMedia::Vector(_)));
    }

    #[test]
    fn unsupported_bytes_error() {
        let registry = MediaDecoderRegistry::with_defaults();
        let err = registry.decode(None, b"not an image at all").unwrap_err();
        assert_eq!(err, ImageDecodeError::UnsupportedFormat);
    }

    #[test]
    fn empty_registry_is_unsupported() {
        let registry = MediaDecoderRegistry::new();
        assert_eq!(
            registry
                .decode(Some("image/png"), &encode(ImageFormat::Png))
                .unwrap_err(),
            ImageDecodeError::UnsupportedFormat
        );
    }

    #[test]
    fn rgba8_length_is_validated() {
        assert!(DecodedImage::new_rgba8(2, 2, vec![0; 16]).is_ok());
        assert!(matches!(
            DecodedImage::new_rgba8(2, 2, vec![0; 15]),
            Err(ImageDecodeError::Decode(_))
        ));
    }
}
