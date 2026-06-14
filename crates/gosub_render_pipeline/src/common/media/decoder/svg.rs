use super::{DecodedMedia, ImageDecodeError, MediaDecoder};
use resvg::usvg;
use std::sync::{Arc, OnceLock};

/// Number of leading bytes scanned when sniffing for an SVG root element.
const SVG_SNIFF_LEN: usize = 1024;

/// Return `usvg::Options` backed by a shared fontdb that has system fonts loaded.
///
/// The database is built once the first time this is called and then reused, so system font
/// discovery only happens once per process.
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

/// Parses SVG into a retained `usvg::Tree`. Unlike raster decoders it does not rasterize —
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
        // SVG is XML text. Scan a bounded prefix for the `<svg` root element (case-insensitive,
        // no allocation) so an optional XML declaration / doctype / BOM ahead of it does not
        // hide the match.
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
