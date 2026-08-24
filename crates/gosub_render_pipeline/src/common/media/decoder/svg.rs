use super::{DecodedMedia, ImageDecodeError, MediaDecoder};
use resvg::usvg;
use std::sync::{Arc, OnceLock};

/// Number of leading bytes scanned when sniffing for an SVG root element.
const SVG_SNIFF_LEN: usize = 1024;

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
        let tree =
            usvg::Tree::from_data(bytes, &self.options()).map_err(|e| ImageDecodeError::Decode(e.to_string()))?;
        Ok(DecodedMedia::Vector(Box::new(tree)))
    }
}
