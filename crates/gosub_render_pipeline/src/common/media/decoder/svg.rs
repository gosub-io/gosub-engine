use super::{DecodedMedia, ImageDecodeError, MediaDecoder};
use gosub_shared::svg_limits::{xml_nesting_depth_exceeds, MAX_SVG_NESTING_DEPTH, SVG_PARSE_STACK_SIZE};
use resvg::usvg;
use std::sync::{Arc, OnceLock};

/// Number of leading bytes scanned when sniffing for an SVG root element.
const SVG_SNIFF_LEN: usize = 1024;

/// Leading bytes of a gzip member, i.e. an SVGZ file.
const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

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
        Ok(DecodedMedia::Vector(Box::new(parse_svg(bytes)?)))
    }
}

/// Parse SVG bytes into a `usvg::Tree`, bounding both nesting depth and stack.
///
/// `usvg::Tree::from_data` would do the gzip step for us, but then the depth check would run on
/// the compressed bytes and a few hundred bytes of SVGZ would walk straight past it. Hence
/// decompress, measure, parse, in that order.
fn parse_svg(bytes: &[u8]) -> Result<usvg::Tree, ImageDecodeError> {
    let decompressed;
    let bytes = if bytes.starts_with(&GZIP_MAGIC) {
        decompressed = usvg::decompress_svgz(bytes).map_err(|e| ImageDecodeError::Decode(e.to_string()))?;
        decompressed.as_slice()
    } else {
        bytes
    };

    // Has to happen before `from_str`: the recursion it bounds is inside the parser.
    if xml_nesting_depth_exceeds(bytes, MAX_SVG_NESTING_DEPTH) {
        return Err(ImageDecodeError::Decode(format!(
            "SVG nests elements deeper than the {MAX_SVG_NESTING_DEPTH} level limit"
        )));
    }

    let text = std::str::from_utf8(bytes).map_err(|_| ImageDecodeError::Decode("SVG is not valid UTF-8".into()))?;

    // Own thread, so the depth limit above translates into an actual stack budget. Callers
    // cannot provide one: `<img src=…svg>` decodes on a default-stack fetch thread, an inline
    // `<svg>` decodes partway down a recursive layout walk on a tokio worker. `svg_options()`
    // is built inside the closure because `usvg::Options` holds non-`Send` resolver closures;
    // the fontdb behind it is a `OnceLock`-shared `Arc`, so system fonts are still scanned once.
    std::thread::scope(|scope| {
        let parse = std::thread::Builder::new()
            .name("svg-parse".into())
            .stack_size(SVG_PARSE_STACK_SIZE)
            .spawn_scoped(scope, || usvg::Tree::from_str(text, &svg_options()))
            .map_err(|e| ImageDecodeError::Decode(format!("could not spawn SVG parse thread: {e}")))?;

        parse
            .join()
            .map_err(|_| ImageDecodeError::Decode("SVG parser panicked".into()))?
            .map_err(|e| ImageDecodeError::Decode(e.to_string()))
    })
}
