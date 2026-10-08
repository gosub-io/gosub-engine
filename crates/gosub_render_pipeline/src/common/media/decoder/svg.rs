use super::{DecodedMedia, ImageDecodeError, MediaDecoder};
use gosub_shared::svg_limits::{
    xml_exceeds_limits, XmlLimit, MAX_NESTED_SVG_DOCUMENTS, MAX_NESTED_SVG_LEVELS, MAX_SVG_INFLATED_BYTES,
    MAX_SVG_NESTING_DEPTH, SVG_PARSE_STACK_NEEDED, SVG_PARSE_STACK_SIZE,
};
use resvg::usvg;
use std::borrow::Cow;
use std::io::Read;
use std::sync::atomic::{AtomicUsize, Ordering};
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

    /// Options for one decode. `budget` is that decode's, shared with every document nested in
    /// it, so a fresh one per decode.
    fn options(&self, budget: &Arc<Budget>) -> usvg::Options<'static> {
        let fontdb = match self.fonts {
            Fonts::System => system_fontdb(),
            Fonts::None => Arc::new(usvg::fontdb::Database::new()),
        };
        usvg::Options {
            fontdb,
            image_href_resolver: usvg::ImageHrefResolver {
                // usvg's default hands a nested SVG straight to its own parser, past the depth
                // scan, the inflate cap and the stack growth below.
                resolve_data: data_resolver(Arc::clone(budget)),
                // usvg's default treats any other `href` as a path and reads
                // it from disk: page content naming `/etc/passwd` would be
                // read by the broker, and stat'd then SIGSYS'd in a confined
                // renderer. An `<image>` in SVG an engine embeds is `data:`
                // or nothing; the fetched kind goes through the media store.
                resolve_string: Box::new(|_, _| None),
            },
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
        Ok(DecodedMedia::Vector(Box::new(parse_svg(bytes, self)?)))
    }
}

/// Parse SVG bytes into a `usvg::Tree`, bounding nesting depth, stack and amplification.
fn parse_svg(bytes: &[u8], decoder: &SvgDecoder) -> Result<usvg::Tree, ImageDecodeError> {
    parse_svg_within(bytes, decoder, Arc::new(Budget::new(MAX_SVG_INFLATED_BYTES)))
}

/// [`parse_svg`] against a given budget, so tests can lower the byte limit.
fn parse_svg_within(bytes: &[u8], decoder: &SvgDecoder, budget: Arc<Budget>) -> Result<usvg::Tree, ImageDecodeError> {
    let options = decoder.options(&budget);
    // The outer document's own bytes arrived over the wire and are bounded there; only what
    // inflating them adds is charged.
    let text = checked_text(bytes, &budget, false).map_err(|r| ImageDecodeError::Decode(r.into_message()))?;
    let text = std::str::from_utf8(&text).map_err(|_| ImageDecodeError::Decode("SVG is not valid UTF-8".into()))?;

    // Grow the stack rather than move to a thread: the depth limit above bounds the number of
    // frames, and this gives them somewhere to sit without the caller having to provide it.
    // `<img src=…svg>` decodes on a fetch thread, but an inline `<svg>` decodes partway down a
    // recursive layout walk on a tokio worker, where the headroom left is anyone's guess. Only
    // allocates when the remaining stack is under `SVG_PARSE_STACK_NEEDED`, so the common case
    // - a shallow document with plenty of stack - is a pointer comparison.
    let tree = stacker::maybe_grow(SVG_PARSE_STACK_NEEDED, SVG_PARSE_STACK_SIZE, || {
        usvg::Tree::from_str(text, &options).map_err(|e| ImageDecodeError::Decode(e.to_string()))
    })?;

    // usvg drops an image its resolver returns nothing for and carries on, so a nested document
    // that broke a limit only shows up here.
    match budget.refused.get() {
        Some(reason) => Err(ImageDecodeError::Decode(reason.clone())),
        None => Ok(tree),
    }
}

/// Why a document's bytes were not handed to the parser.
enum Rejection {
    /// Broke one of the `svg_limits`: hostile, so the whole decode fails.
    Limit(String),
    /// Merely not SVG. A nested image like that is not drawn, as usvg would have it.
    Malformed(String),
}

impl Rejection {
    fn into_message(self) -> String {
        match self {
            Rejection::Limit(m) | Rejection::Malformed(m) => m,
        }
    }
}

/// The SVG text in `bytes`, inflated if it is SVGZ and depth-scanned, charging `budget` for what
/// inflating produced - and, for a `nested` document, for the text itself, since `<use>` can
/// make usvg parse the same `data:` payload once per instance.
///
/// `usvg::Tree::from_data` would do the gzip step for us, but then the depth check would run on
/// the compressed bytes and a few hundred bytes of SVGZ would walk straight past it. Hence
/// decompress, measure, parse, in that order.
fn checked_text<'a>(bytes: &'a [u8], budget: &Budget, nested: bool) -> Result<Cow<'a, [u8]>, Rejection> {
    let text = if bytes.starts_with(&GZIP_MAGIC) {
        let inflated = inflate_svgz(bytes, budget)?;
        budget.charge(inflated.len())?;
        Cow::Owned(inflated)
    } else {
        if nested {
            budget.charge(bytes.len())?;
        }
        Cow::Borrowed(bytes)
    };
    // SVGZ is one gzip layer. A second would be inflated again, uncapped, by
    // `usvg::Tree::from_data_nested`, and is not SVG text in any case.
    if text.starts_with(&GZIP_MAGIC) {
        return Err(Rejection::Malformed("SVGZ inflates to more gzip, not SVG".into()));
    }

    // Has to happen before the parse: the recursion it bounds is inside the parser.
    match xml_exceeds_limits(&text, MAX_SVG_NESTING_DEPTH) {
        Some(XmlLimit::Depth) => Err(Rejection::Limit(format!(
            "SVG nests elements deeper than the {MAX_SVG_NESTING_DEPTH} level limit"
        ))),
        Some(XmlLimit::Entities) => Err(Rejection::Limit(
            "SVG declares its own entities, which can expand to unbounded nesting".into(),
        )),
        None => Ok(text),
    }
}

/// Inflate SVGZ, refusing once the output passes what is left of `budget` rather than after.
fn inflate_svgz(bytes: &[u8], budget: &Budget) -> Result<Vec<u8>, Rejection> {
    let limit = budget.remaining();
    let mut out = Vec::new();
    // One byte past the limit is enough to know it was passed, and all that is ever allocated.
    let cap = (limit as u64).saturating_add(1);
    flate2::read::GzDecoder::new(bytes)
        .take(cap)
        .read_to_end(&mut out)
        .map_err(|_| Rejection::Malformed("SVGZ is not valid gzip".into()))?;
    if out.len() > limit {
        return Err(budget.exceeded());
    }
    Ok(out)
}

/// What one decode has spent of the amplification limits in `gosub_shared::svg_limits`, shared by
/// the outer document and every `data:` SVG nested in it. Atomics only because usvg wants its
/// resolver `Sync`; a parse runs on one thread.
struct Budget {
    max_inflated: usize,
    inflated: AtomicUsize,
    documents: AtomicUsize,
    levels: AtomicUsize,
    /// The first limit a nested document broke, for [`parse_svg_within`] to report.
    refused: OnceLock<String>,
}

impl Budget {
    fn new(max_inflated: usize) -> Self {
        Self {
            max_inflated,
            inflated: AtomicUsize::new(0),
            documents: AtomicUsize::new(0),
            levels: AtomicUsize::new(0),
            refused: OnceLock::new(),
        }
    }

    fn remaining(&self) -> usize {
        self.max_inflated.saturating_sub(self.inflated.load(Ordering::Relaxed))
    }

    fn charge(&self, bytes: usize) -> Result<(), Rejection> {
        let total = self.inflated.fetch_add(bytes, Ordering::Relaxed).saturating_add(bytes);
        if total > self.max_inflated {
            return Err(self.exceeded());
        }
        Ok(())
    }

    fn exceeded(&self) -> Rejection {
        Rejection::Limit(format!(
            "SVG inflates past the {} byte limit on decompressed and nested documents",
            self.max_inflated
        ))
    }

    /// Parse a nested `data:` SVG through the same checks as the outer document, plus the
    /// document count and nesting level limits.
    fn parse_nested(&self, data: &[u8], options: &usvg::Options) -> Result<usvg::Tree, Rejection> {
        if self.documents.fetch_add(1, Ordering::Relaxed) >= MAX_NESTED_SVG_DOCUMENTS {
            return Err(Rejection::Limit(format!(
                "SVG embeds more than {MAX_NESTED_SVG_DOCUMENTS} nested SVG documents"
            )));
        }
        let level = Level::enter(&self.levels);
        if level.0 > MAX_NESTED_SVG_LEVELS {
            return Err(Rejection::Limit(format!(
                "SVG nests SVG images deeper than {MAX_NESTED_SVG_LEVELS} levels"
            )));
        }

        let text = checked_text(data, self, true)?;
        // Each level parses on top of the one embedding it, so it needs the same headroom.
        // `from_data_nested` is usvg's own entry for `<image>` documents: it keeps our resolver
        // (so deeper levels come back through here) and drops string hrefs as the spec asks.
        stacker::maybe_grow(SVG_PARSE_STACK_NEEDED, SVG_PARSE_STACK_SIZE, || {
            usvg::Tree::from_data_nested(&text, options).map_err(|e| Rejection::Malformed(e.to_string()))
        })
    }
}

/// One level of nested parse, counted for as long as it lives.
struct Level<'a>(usize, &'a AtomicUsize);

impl<'a> Level<'a> {
    fn enter(levels: &'a AtomicUsize) -> Self {
        Self(levels.fetch_add(1, Ordering::Relaxed) + 1, levels)
    }
}

impl Drop for Level<'_> {
    fn drop(&mut self) {
        self.1.fetch_sub(1, Ordering::Relaxed);
    }
}

/// usvg's default `data:` resolver, except that SVG goes through [`Budget::parse_nested`].
/// Raster kinds are passed through undecoded, as usvg does; the renderer decodes them.
fn data_resolver(budget: Arc<Budget>) -> usvg::ImageHrefDataResolverFn<'static> {
    Box::new(move |mime: &str, data: Arc<Vec<u8>>, options: &usvg::Options| {
        let raster = |format| match format {
            image::ImageFormat::Jpeg => Some(usvg::ImageKind::JPEG(Arc::clone(&data))),
            image::ImageFormat::Png => Some(usvg::ImageKind::PNG(Arc::clone(&data))),
            image::ImageFormat::Gif => Some(usvg::ImageKind::GIF(Arc::clone(&data))),
            image::ImageFormat::WebP => Some(usvg::ImageKind::WEBP(Arc::clone(&data))),
            _ => None,
        };
        match mime {
            "image/jpg" | "image/jpeg" => raster(image::ImageFormat::Jpeg),
            "image/png" => raster(image::ImageFormat::Png),
            "image/gif" => raster(image::ImageFormat::Gif),
            "image/webp" => raster(image::ImageFormat::WebP),
            "image/svg+xml" => nested_svg(&data, options, &budget),
            // A `data:` URL without a type: sniff raster magic, else try it as SVG.
            "text/plain" => image::guess_format(&data)
                .ok()
                .and_then(raster)
                .or_else(|| nested_svg(&data, options, &budget)),
            _ => None,
        }
    })
}

fn nested_svg(data: &[u8], options: &usvg::Options, budget: &Budget) -> Option<usvg::ImageKind> {
    // Once one limit is broken the decode is going to fail; parse nothing more.
    if budget.refused.get().is_some() {
        return None;
    }
    match budget.parse_nested(data, options) {
        Ok(tree) => Some(usvg::ImageKind::SVG(tree)),
        Err(Rejection::Limit(reason)) => {
            let _ = budget.refused.set(reason);
            None
        }
        Err(Rejection::Malformed(_)) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has_image_node(tree: &usvg::Tree) -> bool {
        fn walk(group: &usvg::Group) -> bool {
            group.children().iter().any(|node| match node {
                usvg::Node::Image(_) => true,
                usvg::Node::Group(g) => walk(g),
                _ => false,
            })
        }
        walk(tree.root())
    }

    /// An `<image href>` naming a host file is dropped, never read. The
    /// control shows usvg's own default would have loaded it.
    #[test]
    fn an_image_href_on_disk_is_never_read() {
        let dir = std::env::temp_dir().join(format!("gosub-svg-href-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("secret.png");
        let mut bytes = Vec::new();
        image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255]))
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        std::fs::write(&png, bytes).unwrap();
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="2" height="2"><image href="{}" width="2" height="2"/></svg>"#,
            png.display()
        );

        let control = usvg::Tree::from_str(&svg, &usvg::Options::default()).unwrap();
        assert!(
            has_image_node(&control),
            "control: usvg's default resolver did not load the file"
        );

        let ours = parse_svg(svg.as_bytes(), &SvgDecoder::without_system_fonts()).unwrap();
        assert!(!has_image_node(&ours), "the file on disk was read");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `data:` images still resolve: the one `href` kind page SVG legitimately carries.
    #[test]
    fn a_data_image_href_still_resolves() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><image href="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==" width="1" height="1"/></svg>"#;
        let tree = parse_svg(svg.as_bytes(), &SvgDecoder::without_system_fonts()).unwrap();
        assert!(has_image_node(&tree), "a data: image was dropped");
    }

    /// A 1x1 PNG, base64.
    const PNG_1X1: &str =
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";

    fn b64(bytes: &[u8]) -> String {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        gz.write_all(bytes).unwrap();
        gz.finish().unwrap()
    }

    /// An SVG holding one `<image>` of `href`, plus whatever `extra` markup.
    fn embedding(href: &str, extra: &str) -> String {
        format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="4" height="4"><defs><image id="i" href="{href}" width="4" height="4"/></defs><use xlink:href="#i"/>{extra}</svg>"##
        )
    }

    fn svg_data_url(svg: &[u8]) -> String {
        format!("data:image/svg+xml;base64,{}", b64(svg))
    }

    const LEAF: &str =
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><rect width="4" height="4"/></svg>"#;

    fn has_nested_svg(tree: &usvg::Tree) -> bool {
        fn walk(group: &usvg::Group) -> bool {
            group.children().iter().any(|node| match node {
                usvg::Node::Image(image) => matches!(image.kind(), usvg::ImageKind::SVG(_)),
                usvg::Node::Group(g) => walk(g),
                _ => false,
            })
        }
        walk(tree.root())
    }

    fn decode(svg: &[u8]) -> Result<usvg::Tree, String> {
        parse_svg(svg, &SvgDecoder::without_system_fonts()).map_err(|e| e.to_string())
    }

    /// A `data:` URL without a type is sniffed, as usvg's default resolver does.
    #[test]
    fn an_untyped_raster_data_image_still_resolves() {
        let svg = embedding(&format!("data:;base64,{PNG_1X1}"), "");
        assert!(
            has_image_node(&decode(svg.as_bytes()).unwrap()),
            "an untyped PNG was dropped"
        );
    }

    #[test]
    fn a_shallow_nested_svg_still_decodes() {
        let svg = embedding(&svg_data_url(LEAF.as_bytes()), "");
        assert!(
            has_nested_svg(&decode(svg.as_bytes()).unwrap()),
            "the nested SVG was dropped"
        );

        // Same again as nested SVGZ, inflated well inside the budget.
        let svg = embedding(&svg_data_url(&gzip(LEAF.as_bytes())), "");
        assert!(
            has_nested_svg(&decode(svg.as_bytes()).unwrap()),
            "the nested SVGZ was dropped"
        );
    }

    /// The nested document gets the outer one's depth scan. Before, usvg's default resolver
    /// parsed it unchecked; `tests/svg_depth_limit.rs` has the stack-overflow-sized version.
    #[test]
    fn a_too_deep_nested_svg_is_rejected() {
        let deep = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg">{}<rect/>{}</svg>"#,
            "<g>".repeat(MAX_SVG_NESTING_DEPTH + 8),
            "</g>".repeat(MAX_SVG_NESTING_DEPTH + 8)
        );
        let err = decode(embedding(&svg_data_url(deep.as_bytes()), "").as_bytes()).unwrap_err();
        assert!(err.contains("deeper than"), "{err}");
    }

    /// A small SVGZ inflating past the budget is refused, top level or nested. The budget is
    /// lowered so the fixture stays small; the control shows the same bytes decode under the
    /// real one, so it is the limit doing the refusing.
    #[test]
    fn an_svgz_inflating_past_the_budget_is_rejected() {
        const LIMIT: usize = 16 * 1024;
        let padded = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4">{}<rect width="4" height="4"/></svg>"#,
            " ".repeat(4 * LIMIT)
        );
        let bomb = gzip(padded.as_bytes());
        assert!(bomb.len() < LIMIT / 8, "fixture should be a bomb: {} bytes", bomb.len());
        let decoder = SvgDecoder::without_system_fonts();
        let lowered = || Arc::new(Budget::new(LIMIT));

        let err = parse_svg_within(&bomb, &decoder, lowered()).unwrap_err().to_string();
        assert!(err.contains("inflates past"), "top level: {err}");

        let nested = embedding(&svg_data_url(&bomb), "");
        let err = parse_svg_within(nested.as_bytes(), &decoder, lowered())
            .unwrap_err()
            .to_string();
        assert!(err.contains("inflates past"), "nested: {err}");

        assert!(has_nested_svg(&decode(nested.as_bytes()).unwrap()), "control");
    }

    /// Several small inflations add up against the one budget rather than each getting its own.
    #[test]
    fn nested_inflations_share_one_budget() {
        const LIMIT: usize = 16 * 1024;
        let padded = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4">{}</svg>"#,
            " ".repeat(LIMIT / 3)
        );
        let one = svg_data_url(&gzip(padded.as_bytes()));
        let decoder = SvgDecoder::without_system_fonts();

        let two = embedding(&one, r##"<use xlink:href="#i"/>"##);
        assert!(parse_svg_within(two.as_bytes(), &decoder, Arc::new(Budget::new(LIMIT))).is_ok());

        let four = embedding(&one, &r##"<use xlink:href="#i"/>"##.repeat(3));
        let err = parse_svg_within(four.as_bytes(), &decoder, Arc::new(Budget::new(LIMIT)))
            .unwrap_err()
            .to_string();
        assert!(err.contains("inflates past"), "{err}");
    }

    /// `<use>` parses the referenced image's document once per instance, so fan-out across
    /// documents is counted in instances.
    #[test]
    fn use_fan_out_of_a_nested_svg_is_capped() {
        let href = svg_data_url(LEAF.as_bytes());
        // The `<defs>` image is not rendered; the one `<use>` in `embedding` plus these is.
        let at_cap = embedding(
            &href,
            &r##"<use xlink:href="#i"/>"##.repeat(MAX_NESTED_SVG_DOCUMENTS - 1),
        );
        assert!(
            decode(at_cap.as_bytes()).is_ok(),
            "{MAX_NESTED_SVG_DOCUMENTS} instances"
        );

        let over = embedding(&href, &r##"<use xlink:href="#i"/>"##.repeat(MAX_NESTED_SVG_DOCUMENTS));
        let err = decode(over.as_bytes()).unwrap_err();
        assert!(err.contains("nested SVG documents"), "{err}");
    }

    /// Nested inside nested inside ...: every level comes back through the guarded resolver.
    #[test]
    fn nested_svg_levels_are_capped() {
        let nest = |levels: usize| {
            let mut svg = LEAF.to_string();
            for _ in 0..levels {
                svg = embedding(&svg_data_url(svg.as_bytes()), "");
            }
            svg
        };
        // The outermost document is level zero.
        assert!(decode(nest(MAX_NESTED_SVG_LEVELS).as_bytes()).is_ok());
        let err = decode(nest(MAX_NESTED_SVG_LEVELS + 1).as_bytes()).unwrap_err();
        assert!(err.contains("SVG images deeper than"), "{err}");
    }
}
