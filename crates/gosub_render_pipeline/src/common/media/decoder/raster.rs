use super::{DecodedMedia, ImageDecodeError, MediaDecoder};

/// Decodes every raster format the `image` crate is compiled with (PNG/JPEG/GIF today). `image`
/// sniffs the real format from the bytes, so a wrong MIME hint between raster formats is harmless.
pub struct RasterDecoder;

impl RasterDecoder {
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
        match image::load_from_memory(bytes) {
            Ok(img) => Ok(DecodedMedia::Raster(img.to_rgba8().into())),
            // Browsers tolerate PNGs with bad chunk CRCs (some encoders emit them); the `image`
            // crate rejects them. Retry a PNG with checksum validation disabled so we match
            // browser behavior instead of showing a broken-image placeholder.
            Err(e) if is_png(bytes) => decode_png_lenient(bytes)
                .map(|img| DecodedMedia::Raster(img.into()))
                .map_err(|_| ImageDecodeError::Decode(e.to_string())),
            Err(e) => Err(ImageDecodeError::Decode(e.to_string())),
        }
    }
}

fn is_png(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a])
}

/// Decode a PNG ignoring chunk CRC errors, normalized to 8-bit RGBA - the lenient path browsers
/// use for PNGs with incorrect checksums.
fn decode_png_lenient(bytes: &[u8]) -> anyhow::Result<image::RgbaImage> {
    let mut options = png::DecodeOptions::default();
    options.set_ignore_checksums(true);
    let mut decoder = png::Decoder::new_with_options(std::io::Cursor::new(bytes), options);
    // EXPAND: palette → RGB, sub-8-bit → 8-bit, and tRNS → alpha channel. STRIP_16: 16-bit → 8-bit.
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);

    let mut reader = decoder.read_info()?;
    let mut buf = vec![0u8; reader.output_buffer_size().unwrap_or(0)];
    let info = reader.next_frame(&mut buf)?;
    let src = &buf[..info.buffer_size()];
    let (w, h) = (info.width, info.height);

    // After EXPAND + STRIP_16 the channel layout is one of these 8-bit color types.
    let mut rgba = Vec::with_capacity((w as usize) * (h as usize) * 4);
    match info.color_type {
        png::ColorType::Grayscale => {
            for &g in src {
                rgba.extend_from_slice(&[g, g, g, 255]);
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for ga in src.chunks_exact(2) {
                rgba.extend_from_slice(&[ga[0], ga[0], ga[0], ga[1]]);
            }
        }
        png::ColorType::Rgb => {
            for c in src.chunks_exact(3) {
                rgba.extend_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
        png::ColorType::Rgba => rgba.extend_from_slice(src),
        other => anyhow::bail!("unexpected PNG color type after expansion: {other:?}"),
    }

    image::RgbaImage::from_raw(w, h, rgba).ok_or_else(|| anyhow::anyhow!("PNG buffer size mismatch"))
}
