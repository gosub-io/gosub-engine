use super::{DecodedMedia, ImageDecodeError, MediaDecoder};
use gosub_shared::svg_limits::{
    xml_exceeds_limits, XmlLimit, MAX_SVG_NESTING_DEPTH, SVG_PARSE_STACK_NEEDED, SVG_PARSE_STACK_SIZE,
};
use resvg::usvg;
use std::sync::{Arc, OnceLock};

/// Number of leading bytes scanned when sniffing for an SVG root element.
const SVG_SNIFF_LEN: usize = 1024;

/// Leading bytes of a gzip member, i.e. an SVGZ file.
const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

/// Built once per process: discovery walks every font directory.
static FONTDB: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();

/// The system fontdb, shared so font discovery happens only once per process.
fn system_fontdb() -> Arc<usvg::fontdb::Database> {
    Arc::clone(FONTDB.get_or_init(|| Arc::new(build_system_fontdb(false))))
}

/// Discovery only indexes faces; their bytes are opened lazily on use. With `pin`,
/// every face is mapped now instead, so later use never opens a file.
// Mapping rather than reading: the system font set runs to hundreds of MiB, and
// a mapping costs nothing until a page is touched.
#[allow(unsafe_code)]
fn build_system_fontdb(pin: bool) -> usvg::fontdb::Database {
    let mut db = usvg::fontdb::Database::new();
    db.load_system_fonts();
    if pin {
        let ids: Vec<_> = db.faces().map(|face| face.id).collect();
        for id in ids {
            // SAFETY: the mapped files are system fonts, not written to while
            // this process runs; the same assumption fontdb's own lazy path makes.
            let _ = unsafe { db.make_shared_face_data(id) };
        }
    }
    db
}

/// Where `<text>` conversion gets its fonts.
#[derive(Debug, Clone, Copy)]
enum Fonts {
    System,
    None,
}

/// Parses SVG into a retained `usvg::Tree`. Unlike raster decoders it does not rasterize -
/// the tree is kept so it can be re-rasterized crisply at any render size.
pub struct SvgDecoder {
    fonts: Fonts,
}

impl SvgDecoder {
    pub fn new() -> Self {
        Self { fonts: Fonts::System }
    }

    /// No font discovery at all, so `<text>` converts to nothing. For a process that
    /// only *validates* SVG - the sandboxed decoder, whose parsed tree never leaves it
    /// (the broker re-parses accepted bytes with real fonts) - and whose sandbox forbids
    /// the filesystem walk that discovery and lazy face loading need.
    pub fn without_system_fonts() -> Self {
        Self { fonts: Fonts::None }
    }

    /// Map every system face into memory ahead of a sandbox that forbids opening
    /// files, so `<text>` conversion never goes to disk afterwards. The process's
    /// counterpart to `FontSystem::prepare_for_confinement`. Must precede the first
    /// system-font decode in this process: the database is built once, and this
    /// returns `false` (leaving the lazily-loading one in place) if that already
    /// happened.
    pub fn pin_system_fonts() -> bool {
        FONTDB.set(Arc::new(build_system_fontdb(true))).is_ok()
    }

    /// Find the stack bounds [`parse_svg`] grows from now, so a later parse on this
    /// thread never goes looking for them. Looking reads `/proc/self/maps` (glibc, main
    /// thread), which a sandbox that forbids opening files kills the process for. Call
    /// on the thread that will decode, before the lockdown.
    pub fn prepare_for_confinement() {
        let _ = stacker::remaining_stack();
    }

    fn options(&self) -> usvg::Options<'static> {
        let fontdb = match self.fonts {
            Fonts::System => system_fontdb(),
            Fonts::None => Arc::new(usvg::fontdb::Database::new()),
        };
        usvg::Options {
            fontdb,
            ..Default::default()
        }
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
        Ok(DecodedMedia::Vector(Box::new(parse_svg(bytes, &self.options())?)))
    }
}

/// Parse SVG bytes into a `usvg::Tree`, bounding both nesting depth and stack.
///
/// `usvg::Tree::from_data` would do the gzip step for us, but then the depth check would run on
/// the compressed bytes and a few hundred bytes of SVGZ would walk straight past it. Hence
/// decompress, measure, parse, in that order.
fn parse_svg(bytes: &[u8], options: &usvg::Options<'static>) -> Result<usvg::Tree, ImageDecodeError> {
    let decompressed;
    let bytes = if bytes.starts_with(&GZIP_MAGIC) {
        decompressed = usvg::decompress_svgz(bytes).map_err(|e| ImageDecodeError::Decode(e.to_string()))?;
        decompressed.as_slice()
    } else {
        bytes
    };

    // Has to happen before `from_str`: the recursion it bounds is inside the parser.
    match xml_exceeds_limits(bytes, MAX_SVG_NESTING_DEPTH) {
        Some(XmlLimit::Depth) => {
            return Err(ImageDecodeError::Decode(format!(
                "SVG nests elements deeper than the {MAX_SVG_NESTING_DEPTH} level limit"
            )))
        }
        Some(XmlLimit::Entities) => {
            return Err(ImageDecodeError::Decode(
                "SVG declares its own entities, which can expand to unbounded nesting".into(),
            ))
        }
        None => {}
    }

    let text = std::str::from_utf8(bytes).map_err(|_| ImageDecodeError::Decode("SVG is not valid UTF-8".into()))?;

    // Grow the stack rather than move to a thread: the depth limit above bounds the number of
    // frames, and this gives them somewhere to sit without the caller having to provide it.
    // `<img src=…svg>` decodes on a fetch thread, but an inline `<svg>` decodes partway down a
    // recursive layout walk on a tokio worker, where the headroom left is anyone's guess. Only
    // allocates when the remaining stack is under `SVG_PARSE_STACK_NEEDED`, so the common case
    // - a shallow document with plenty of stack - is a pointer comparison.
    stacker::maybe_grow(SVG_PARSE_STACK_NEEDED, SVG_PARSE_STACK_SIZE, || {
        usvg::Tree::from_str(text, options).map_err(|e| ImageDecodeError::Decode(e.to_string()))
    })
}
