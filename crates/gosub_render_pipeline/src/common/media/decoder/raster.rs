use super::{DecodedImage, DecodedMedia, ImageDecodeError, MediaDecoder};

/// Decodes every raster format the `image` crate is compiled with (PNG/JPEG/GIF today; more
/// when extra `image` features are enabled). The actual format is sniffed by the `image`
/// crate from the bytes, so a wrong MIME hint between raster formats is harmless.
pub struct RasterDecoder;

impl RasterDecoder {
    /// Strip any `;`-delimited parameters and return the trimmed essence of a MIME type.
    fn essence(mime: &str) -> &str {
        mime.split(';').next().unwrap_or(mime).trim()
    }
}

impl MediaDecoder for RasterDecoder {
    fn name(&self) -> &'static str {
        "raster"
    }

    fn supports_mime(&self, mime: &str) -> bool {
        let mime = Self::essence(mime);
        // Any `image/*` type except SVG, which is a vector format handled elsewhere.
        mime.len() >= 6
            && mime[..6].eq_ignore_ascii_case("image/")
            && !mime.eq_ignore_ascii_case("image/svg+xml")
            && !mime.eq_ignore_ascii_case("image/svg")
    }

    fn supports_magic(&self, bytes: &[u8]) -> bool {
        image::guess_format(bytes).is_ok()
    }

    fn decode(&self, bytes: &[u8]) -> Result<DecodedMedia, ImageDecodeError> {
        let img = image::load_from_memory(bytes)
            .map_err(|e| ImageDecodeError::Decode(e.to_string()))?
            .to_rgba8();
        let (width, height) = img.dimensions();
        let decoded = DecodedImage::new_rgba8(width, height, img.into_raw())?;
        Ok(DecodedMedia::Raster(decoded))
    }
}
