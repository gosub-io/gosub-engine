use super::{DecodedMedia, ImageDecodeError, MediaDecoder};
use resvg::usvg;
use std::sync::{Arc, OnceLock};

/// Number of leading bytes scanned when sniffing for an SVG root element.
const SVG_SNIFF_LEN: usize = 1024;

/// `usvg::Options` backed by a shared fontdb, built once and reused so system font discovery
/// happens only once per process.
fn svg_options() -> usvg::Options<'static> {
    static FONTDB: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    let fontdb = Arc::clone(FONTDB.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        Arc::new(db)
    }));
    usvg::Options {
        fontdb,
        ..Default::default()
    }
}

/// Parses SVG into a retained `usvg::Tree`. Unlike raster decoders it does not rasterize -
/// the tree is kept so it can be re-rasterized crisply at any render size.
pub struct SvgDecoder;

impl SvgDecoder {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SvgDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl MediaDecoder for SvgDecoder {
    fn name(&self) -> &'static str {
        "svg"
    }

    fn supports_mime(&self, mime: &str) -> bool {
        let mime = mime.split(';').next().unwrap_or(mime).trim();
        // `image/svg+xml` is the standard type; `image/svg` is a non-standard alias some
        // servers emit.
        mime.eq_ignore_ascii_case("image/svg+xml") || mime.eq_ignore_ascii_case("image/svg")
    }

    fn supports_magic(&self, bytes: &[u8]) -> bool {
        // Scan a bounded prefix rather than matching at offset 0: an XML declaration, doctype or
        // BOM can precede the `<svg` root.
        const NEEDLE: &[u8] = b"<svg";
        let len = bytes.len().min(SVG_SNIFF_LEN);
        bytes[..len]
            .windows(NEEDLE.len())
            .any(|w| w.eq_ignore_ascii_case(NEEDLE))
    }

    fn decode(&self, bytes: &[u8]) -> Result<DecodedMedia, ImageDecodeError> {
        let tree = usvg::Tree::from_data(bytes, &svg_options()).map_err(|e| ImageDecodeError::Decode(e.to_string()))?;
        Ok(DecodedMedia::Vector(Box::new(tree)))
    }
}
