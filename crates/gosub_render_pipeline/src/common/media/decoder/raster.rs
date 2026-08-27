use super::{DecodedImage, DecodedMedia, ImageDecodeError, MediaDecoder};

/// Largest edge a raster image may have before it is refused outright.
pub const MAX_IMAGE_EDGE: u32 = 16_384;
/// Most memory one decode may allocate before it is refused (the `image` crate's own limit).
pub const MAX_DECODE_BYTES: u64 = 128 * 1024 * 1024;
/// Pixels kept per image: anything larger is downscaled (keeping its intrinsic size for
/// layout), so a page of photographs holds at most `MAX_KEPT_PIXELS * 4` bytes per image
/// rather than whatever the camera produced. 4 Mpx is 2048² - past what a viewport shows.
pub const MAX_KEPT_PIXELS: u64 = 4 * 1024 * 1024;

/// Decodes every raster format the `image` crate is compiled with (PNG/JPEG/GIF today). `image`
/// sniffs the real format from the bytes, so a wrong MIME hint between raster formats is harmless.
/// Decoding is bounded ([`MAX_IMAGE_EDGE`], [`MAX_DECODE_BYTES`]) and huge images are kept
/// downscaled ([`MAX_KEPT_PIXELS`]): a page cannot exhaust a renderer's memory with photographs,
/// and an absurd header fails cleanly instead of allocating.
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
        match decode_bounded(bytes) {
            Ok(img) => Ok(DecodedMedia::Raster(bounded(img))),
            // Browsers tolerate PNGs with bad chunk CRCs (some encoders emit them); the `image`
            // crate rejects them. Retry a PNG with checksum validation disabled so we match
            // browser behavior instead of showing a broken-image placeholder.
            Err(e) if is_png(bytes) => decode_png_lenient(bytes)
                .map(|img| DecodedMedia::Raster(bounded(img)))
                .map_err(|_| ImageDecodeError::Decode(e.to_string())),
            Err(e) => Err(ImageDecodeError::Decode(e.to_string())),
        }
    }

    fn dimensions(&self, bytes: &[u8]) -> Option<(u32, u32)> {
        let (width, height) = image::ImageReader::new(std::io::Cursor::new(bytes))
            .with_guessed_format()
            .ok()?
            .into_dimensions()
            .ok()?;
        // The same bound `decode` enforces: an absurd header is not a size for layout.
        (width <= MAX_IMAGE_EDGE && height <= MAX_IMAGE_EDGE).then_some((width, height))
    }
}

/// Decode with the size and allocation limits applied by the decoder itself, so an
/// oversized header is refused before anything is allocated for it.
fn decode_bounded(bytes: &[u8]) -> Result<image::RgbaImage, image::ImageError> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_EDGE);
    limits.max_image_height = Some(MAX_IMAGE_EDGE);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);
    Ok(reader.decode()?.to_rgba8())
}

/// Keep at most [`MAX_KEPT_PIXELS`] of a decoded image, remembering its real size for layout.
fn bounded(img: image::RgbaImage) -> DecodedImage {
    let (w, h) = img.dimensions();
    let pixels = u64::from(w) * u64::from(h);
    if pixels <= MAX_KEPT_PIXELS {
        return img.into();
    }
    let scale = (MAX_KEPT_PIXELS as f64 / pixels as f64).sqrt();
    let (tw, th) = (
        ((f64::from(w) * scale) as u32).max(1),
        ((f64::from(h) * scale) as u32).max(1),
    );
    let small = image::imageops::resize(&img, tw, th, image::imageops::FilterType::Triangle);
    drop(img);
    DecodedImage::from(small).with_intrinsic(w, h)
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
    let (w, h) = (reader.info().width, reader.info().height);
    if w > MAX_IMAGE_EDGE || h > MAX_IMAGE_EDGE || u64::from(w) * u64::from(h) * 4 > MAX_DECODE_BYTES {
        anyhow::bail!("PNG of {w}x{h} exceeds the decode limits");
    }
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
            for ga in src.as_chunks::<2>().0 {
                rgba.extend_from_slice(&[ga[0], ga[0], ga[0], ga[1]]);
            }
        }
        png::ColorType::Rgb => {
            for c in src.as_chunks::<3>().0 {
                rgba.extend_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
        png::ColorType::Rgba => rgba.extend_from_slice(src),
        other => anyhow::bail!("unexpected PNG color type after expansion: {other:?}"),
    }

    image::RgbaImage::from_raw(w, h, rgba).ok_or_else(|| anyhow::anyhow!("PNG buffer size mismatch"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_of(width: u32, height: u32) -> Vec<u8> {
        use image::ImageEncoder;
        let pixels = vec![0x80u8; (width * height * 4) as usize];
        let mut out = Vec::new();
        image::codecs::png::PngEncoder::new(&mut out)
            .write_image(&pixels, width, height, image::ExtendedColorType::Rgba8)
            .expect("encode");
        out
    }

    #[test]
    fn huge_images_are_kept_downscaled_but_lay_out_at_their_own_size() {
        let bytes = png_of(4000, 3000);
        let DecodedMedia::Raster(img) = RasterDecoder.decode(&bytes).expect("decode") else {
            panic!("expected a raster image");
        };
        assert_eq!((img.intrinsic_width(), img.intrinsic_height()), (4000, 3000));
        assert!(u64::from(img.width()) * u64::from(img.height()) <= MAX_KEPT_PIXELS);
        assert!(img.width() < 4000 && img.height() < 3000);
        // Aspect ratio survives the downscale.
        let ratio = f64::from(img.width()) / f64::from(img.height());
        assert!((ratio - 4.0 / 3.0).abs() < 0.01, "ratio {ratio}");
    }

    #[test]
    fn dimensions_come_from_the_header() {
        assert_eq!(RasterDecoder.dimensions(&png_of(300, 20)), Some((300, 20)));
        assert_eq!(RasterDecoder.dimensions(b"not an image"), None);
    }

    #[test]
    fn small_images_are_untouched() {
        let bytes = png_of(300, 200);
        let DecodedMedia::Raster(img) = RasterDecoder.decode(&bytes).expect("decode") else {
            panic!("expected a raster image");
        };
        assert_eq!((img.width(), img.height()), (300, 200));
        assert_eq!((img.intrinsic_width(), img.intrinsic_height()), (300, 200));
    }

    #[test]
    fn an_absurd_header_is_refused_before_allocating() {
        // A valid PNG signature and IHDR claiming 100k x 100k: 40 GB of RGBA.
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        let ihdr: [u8; 13] = [0, 1, 0x86, 0xa0, 0, 1, 0x86, 0xa0, 8, 6, 0, 0, 0];
        bytes.extend_from_slice(&13u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&ihdr);
        bytes.extend_from_slice(&[0, 0, 0, 0]); // wrong CRC; the lenient path must refuse too
        assert!(RasterDecoder.decode(&bytes).is_err());
    }
}
