//! `@font-face` web fonts: found in the document's stylesheets, fetched through the zone's
//! fetcher, and registered with the font system before the document is handed to the tab.
//!
//! This used to run on the tab worker, synchronously, with a blocking fetch per font -- so a
//! slow font server stopped the tab answering commands or drawing frames, and none of those
//! requests went through the fetcher, which meant no policy, no cache and nothing in the
//! network panel. Here the same work is a stage of the parse, where waiting costs a task
//! rather than a tab.

use crate::html::{EngineDocument, RenderConfiguration};
use gosub_interface::css3::CssStylesheet as _;
use gosub_interface::document::Document as _;
use gosub_interface::font_system::FontSystem as _;
use parking_lot::Mutex;
use std::collections::HashSet;
use std::sync::Arc;
use url::Url;

use super::html::{fetch_subresource, SubFetch};

/// One `@font-face` to load: the CSS family it names, and the sources to try in order.
struct Face {
    family: String,
    sources: Vec<Url>,
}

/// Fetch and register every web font the document's stylesheets declare.
///
/// Faces load concurrently; the sources within a face are tried in order and stop at the
/// first that registers, because they are alternatives to each other (`src: url(a), url(b)`)
/// rather than a list of things to fetch. A source that fails at any step - fetch, decode, or
/// the font system refusing it - falls through to the next (CSS Fonts 4, `src`: "if the
/// resource ... is invalid, the user agent must proceed to the next" source).
pub(crate) async fn load_web_fonts<C: RenderConfiguration>(
    doc: &EngineDocument<C>,
    base_url: &Url,
    font_system: &Arc<Mutex<C::FontSystem>>,
    fetch: &SubFetch<'_>,
    timing_scope: Option<gosub_shared::timing::ScopeId>,
) {
    let faces = collect_faces::<C>(doc, base_url);
    if faces.is_empty() {
        return;
    }

    let fetch_font = |url: Url| async move {
        let started = std::time::Instant::now();
        let body = fetch_subresource(url.as_str(), crate::net::types::ResourceKind::Font, fetch).await;
        let elapsed = started.elapsed().as_micros() as u64;
        // Recorded against the navigation explicitly: this runs across awaits, where a
        // thread-local scope does not hold.
        match timing_scope {
            Some(scope) => gosub_shared::timing::record_in(
                scope,
                gosub_shared::timing::Timing::NetFetchFont,
                elapsed,
                Some(url.to_string()),
            ),
            None => gosub_shared::timing::record(
                gosub_shared::timing::Timing::NetFetchFont,
                elapsed,
                Some(url.to_string()),
            ),
        }
        body.map(|(_, bytes)| bytes)
    };

    let loaded = futures_util::future::join_all(faces.iter().map(|face| load_face(face, 0, &fetch_font))).await;

    // Registration is serialised behind the font system's lock anyway, and doing it here --
    // in the order the document declared the faces -- keeps the result independent of which
    // fetch happened to finish first.
    for (face, first) in faces.iter().zip(loaded) {
        register_face(face, first, &fetch_font, &mut |bytes, family| {
            font_system.lock().register_font(bytes, Some(family))
        })
        .await;
    }
}

/// Register `face` from `candidate` (what [`load_face`] found), going on to the face's next
/// source whenever the font system refuses one: the rare path, one source at a time, not
/// worth fetching ahead for. Whether any source registered.
async fn register_face<F, Fut, R>(
    face: &Face,
    mut candidate: Option<(usize, Url, Vec<u8>)>,
    fetch: &F,
    register: &mut R,
) -> bool
where
    F: Fn(Url) -> Fut,
    Fut: std::future::Future<Output = Option<Vec<u8>>>,
    R: FnMut(Vec<u8>, &str) -> Result<(), gosub_interface::font::FontError>,
{
    while let Some((index, url, bytes)) = candidate {
        match register(bytes, &face.family) {
            Ok(()) => {
                log::debug!("Registered web font '{}' from {url}", face.family);
                return true;
            }
            Err(e) => log::warn!("Failed to register web font '{}' from {url}: {e:?}", face.family),
        }
        candidate = load_face(face, index + 1, fetch).await;
    }
    false
}

/// A blocking fetch: the bytes and their content type, or nothing.
#[cfg(all(feature = "process-isolation", target_os = "linux"))]
type FetchBlocking<'a> = &'a dyn Fn(&str) -> Option<(Option<String>, Vec<u8>)>;

/// Hands a fetched face to the font system: `(bytes, css_family)`.
#[cfg(all(feature = "process-isolation", target_os = "linux"))]
type RegisterFont<'a> = &'a mut dyn FnMut(Vec<u8>, &str) -> Result<(), gosub_interface::font::FontError>;

/// [`load_web_fonts`] for a caller with no runtime to await on: a renderer process, whose
/// every fetch is a blocking round trip to the broker. `register` hands each face to the font
/// system under its CSS family.
#[cfg(all(feature = "process-isolation", target_os = "linux"))]
pub(crate) fn load_web_fonts_blocking<C: RenderConfiguration>(
    doc: &EngineDocument<C>,
    base_url: &Url,
    fetch: FetchBlocking<'_>,
    register: RegisterFont<'_>,
) {
    for face in collect_faces::<C>(doc, base_url) {
        // Each source in turn until one fetches, decodes and registers, as `load_web_fonts`.
        for url in &face.sources {
            let bytes = match fetch(url.as_str()) {
                Some((_, bytes)) if !bytes.is_empty() => bytes,
                _ => {
                    log::warn!("Web font fetch {url} produced nothing");
                    continue;
                }
            };
            let Some(bytes) = decode_web_font(bytes, url) else {
                continue;
            };
            match register(bytes, &face.family) {
                Ok(()) => {
                    log::debug!("Registered web font '{}' from {url}", face.family);
                    break;
                }
                Err(e) => log::warn!("Failed to register web font '{}' from {url}: {e:?}", face.family),
            }
        }
    }
}

/// Every face worth loading, in document order, deduplicated by source URL.
fn collect_faces<C: RenderConfiguration>(doc: &EngineDocument<C>, base_url: &Url) -> Vec<Face> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut faces = Vec::new();

    for sheet in doc.stylesheets() {
        let sheet_url = Url::parse(sheet.url()).ok();
        for (family, sources, unicode_range) in sheet.font_faces() {
            // Google-style web fonts split a family into many `unicode-range` subsets (latin,
            // cyrillic, greek, ...). We don't do per-glyph subset fallback, so register only
            // subsets covering Basic Latin (and ranges with no descriptor), which covers
            // Latin-script content without piling unusable subsets onto the same family.
            if let Some(range) = &unicode_range {
                if !unicode_range_covers_basic_latin(range) {
                    continue;
                }
            }

            let urls: Vec<Url> = sources
                .iter()
                .filter_map(|src| {
                    let base = sheet_url.as_ref().unwrap_or(base_url);
                    base.join(src).or_else(|_| base_url.join(src)).ok()
                })
                // A URL another face already claimed is that face's to register; a second
                // family pointing at the same file would only fetch it twice.
                .filter(|url| seen.insert(url.to_string()))
                .collect();

            if !urls.is_empty() {
                faces.push(Face { family, sources: urls });
            }
        }
    }
    faces
}

/// Try a face's sources in order, from `start`, and return the first that fetched and decoded
/// to a usable font, with its index so a caller whose font system refuses it can go on from
/// the next.
async fn load_face<F, Fut>(face: &Face, start: usize, fetch: &F) -> Option<(usize, Url, Vec<u8>)>
where
    F: Fn(Url) -> Fut,
    Fut: std::future::Future<Output = Option<Vec<u8>>>,
{
    for (index, url) in face.sources.iter().enumerate().skip(start) {
        match fetch(url.clone()).await {
            Some(bytes) if !bytes.is_empty() => match decode_web_font(bytes, url) {
                Some(font) => return Some((index, url.clone(), font)),
                // Rejected and logged there; on to the next source.
                None => continue,
            },
            _ => log::warn!("Web font fetch {url} produced nothing"),
        }
    }
    None
}

/// Whether a CSS `unicode-range` descriptor (e.g. `"U+0000-00FF, U+0131"`) includes the
/// Basic-Latin letter `U+0041` ('A') - our proxy for "covers Latin-script text".
fn unicode_range_covers_basic_latin(range: &str) -> bool {
    const TARGET: u32 = 0x41; // 'A'
    for token in range.split([',', ' ', '\t', '\n', '\r']).filter(|t| !t.is_empty()) {
        let Some(hex) = token
            .trim()
            .strip_prefix("U+")
            .or_else(|| token.trim().strip_prefix("u+"))
        else {
            continue;
        };
        let (lo, hi) = match hex.split_once('-') {
            Some((a, b)) => (parse_hex_bound(a, false), parse_hex_bound(b, true)),
            None => (parse_hex_bound(hex, false), parse_hex_bound(hex, true)),
        };
        if let (Some(lo), Some(hi)) = (lo, hi) {
            if lo <= TARGET && TARGET <= hi {
                return true;
            }
        }
    }
    false
}

/// The largest font a web font may unpack to. Matches the 30 MiB limit of the reference WOFF2
/// decoder (`kDefaultMaxSize`), which Chromium (via OTS) and FreeType also apply; real web
/// fonts, even CJK ones, stay well under it.
const MAX_WEB_FONT_SIZE: u64 = 30 * 1024 * 1024;

/// Unwrap a downloaded web-font payload into raw SFNT bytes the font backends can decode.
///
/// WOFF2 (magic `wOF2`) is a Brotli-compressed wrapper around an OpenType/TrueType font,
/// with the `glyf`/`loca` tables stored in a transformed form. Skia and fontconfig don't
/// decode it (e.g. Google Fonts serves WOFF2 to modern UAs like ours), so we decompress it
/// to a flat SFNT here. Bare SFNT (`OTTO`/`true`/`ttcf`/`0x00010000`) and anything we don't
/// recognise are returned unchanged - including WOFF1, which the backends already handle.
/// On a decode error we log and return the original bytes so the subsequent `register_font`
/// surfaces a single, consistent failure path.
///
/// A WOFF or WOFF2 font that unpacks past [`MAX_WEB_FONT_SIZE`] is a decompression bomb, not
/// a font: it returns `None` and the face is skipped, so neither we nor a backend inflates it.
fn decode_web_font(bytes: Vec<u8>, font_url: &Url) -> Option<Vec<u8>> {
    const WOFF_MAGIC: &[u8; 4] = b"wOFF";
    const WOFF2_MAGIC: &[u8; 4] = b"wOF2";
    if bytes.len() >= 4 && &bytes[0..4] == WOFF_MAGIC {
        if let Err(e) = check_woff_size(&bytes, MAX_WEB_FONT_SIZE) {
            log::warn!("Rejected WOFF web font from {font_url}: {e}");
            return None;
        }
        return Some(bytes);
    }
    if bytes.len() < 4 || &bytes[0..4] != WOFF2_MAGIC {
        return Some(bytes);
    }
    if let Err(e) = check_woff2_size(&bytes, MAX_WEB_FONT_SIZE) {
        log::warn!("Rejected WOFF2 web font from {font_url}: {e}");
        return None;
    }
    match woff2_to_sfnt(&bytes) {
        Ok(sfnt) => {
            if let Err(e) = check_sfnt_size(sfnt.len(), MAX_WEB_FONT_SIZE) {
                log::warn!("Rejected WOFF2 web font from {font_url}: {e}");
                return None;
            }
            log::debug!(
                "Decoded WOFF2 web font from {font_url} ({} → {} bytes)",
                bytes.len(),
                sfnt.len()
            );
            Some(sfnt)
        }
        Err(e) => {
            log::warn!("Failed to decode WOFF2 web font from {font_url}: {e}");
            Some(bytes)
        }
    }
}

/// Check that the SFNT rebuilt from a WOFF2 font is at most `cap` bytes. The Brotli stream is
/// already capped, but rebuilding adds the header and table directory, and the transformed
/// `glyf`/`loca` tables expand when they are reconstructed, so the result can still exceed it.
fn check_sfnt_size(len: usize, cap: u64) -> Result<(), String> {
    if len as u64 > cap {
        return Err(format!("rebuilt font is {len} bytes, over the {cap} byte cap"));
    }
    Ok(())
}

/// Check that a WOFF1 font unpacks to at most `cap` bytes. The backends (FreeType) size each
/// table's zlib inflate by its `origLength` and the whole font by `totalSfntSize`, so bounding
/// those header fields bounds what they allocate and inflate.
fn check_woff_size(bytes: &[u8], cap: u64) -> Result<(), String> {
    const HEADER_LEN: usize = 44;
    const ENTRY_LEN: usize = 20;
    let u16_at = |at: usize| bytes.get(at..at + 2).map(|b| u16::from_be_bytes([b[0], b[1]]));
    let u32_at = |at: usize| {
        bytes
            .get(at..at + 4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    };

    let num_tables = u16_at(12).ok_or("truncated header")?;
    let total_sfnt_size = u32_at(16).ok_or("truncated header")?;
    if u64::from(total_sfnt_size) > cap {
        return Err(format!("totalSfntSize {total_sfnt_size} exceeds {cap}"));
    }
    let mut unpacked = 0u64;
    for i in 0..usize::from(num_tables) {
        // origLength is the fourth field of each 20-byte table directory entry.
        let orig_length = u32_at(HEADER_LEN + i * ENTRY_LEN + 12).ok_or("truncated table directory")?;
        unpacked += u64::from(orig_length);
    }
    if unpacked > cap {
        return Err(format!("tables unpack to {unpacked} bytes, over {cap}"));
    }
    Ok(())
}

/// Check that a WOFF2 font unpacks to at most `cap` bytes, before allsorts sees it.
///
/// allsorts inflates the Brotli stream with `read_to_end`, sized by nothing in the header, so
/// a small stream could expand without bound. We inflate it once here into a sink through a
/// `take(cap + 1)` and reject the font if it reaches the cap; only then does allsorts inflate
/// it for real. `totalSfntSize` is checked first so an honest oversized font fails cheaply.
fn check_woff2_size(bytes: &[u8], cap: u64) -> Result<(), String> {
    use allsorts::binary::read::ReadScope;
    use allsorts::woff2::{PackedU16, TableDirectoryEntry, Woff2Header};
    use std::io::Read as _;

    let mut ctxt = ReadScope::new(bytes).ctxt();
    let header = ctxt.read::<Woff2Header>().map_err(|e| format!("header: {e:?}"))?;
    if u64::from(header.total_sfnt_size) > cap {
        return Err(format!("totalSfntSize {} exceeds {cap}", header.total_sfnt_size));
    }

    // Walk past the table directory (and, for a collection, the collection directory) to
    // reach the compressed stream, the same way allsorts' `Woff2Font::read` does.
    for _ in 0..header.num_tables {
        ctxt.read_dep::<TableDirectoryEntry>(0)
            .map_err(|e| format!("table directory: {e:?}"))?;
    }
    if header.flavor == allsorts::tables::TTCF_MAGIC {
        let mut directory = || -> Result<(), allsorts::error::ParseError> {
            let _ttc_version = ctxt.read_u32be()?;
            for _ in 0..ctxt.read::<PackedU16>()? {
                let num_tables = ctxt.read::<PackedU16>()?;
                let _flavor = ctxt.read_u32be()?;
                for _ in 0..num_tables {
                    ctxt.read::<PackedU16>()?;
                }
            }
            Ok(())
        };
        directory().map_err(|e| format!("collection directory: {e:?}"))?;
    }
    let compressed = usize::try_from(header.total_compressed_size)
        .ok()
        .and_then(|len| ctxt.read_slice(len).ok())
        .ok_or("truncated compressed data")?;

    let mut stream = brotli_decompressor::Decompressor::new(compressed, 4096).take(cap + 1);
    let unpacked = std::io::copy(&mut stream, &mut std::io::sink()).map_err(|e| format!("brotli: {e}"))?;
    if unpacked > cap {
        return Err(format!("Brotli stream unpacks past {cap} bytes"));
    }
    Ok(())
}

/// Decompress a WOFF2 font to a flat SFNT (TTF/OTF) byte buffer. allsorts handles the Brotli
/// decompression and the `glyf`/`loca` transform reconstruction; we then re-assemble the
/// reconstructed tables into the on-disk SFNT layout (offset table + table directory + 4-byte
/// aligned table data) that font backends expect.
fn woff2_to_sfnt(bytes: &[u8]) -> Result<Vec<u8>, String> {
    use allsorts::binary::read::ReadScope;
    use allsorts::woff2::Woff2Font;

    let font = ReadScope::new(bytes)
        .read::<Woff2Font<'_>>()
        .map_err(|e| format!("parse: {e:?}"))?;
    let sfnt_version = font.flavor();
    let tables = font
        .table_provider(0)
        .map_err(|e| format!("reconstruct: {e:?}"))?
        .into_tables();

    Ok(assemble_sfnt(sfnt_version, tables))
}

/// Pack a set of font tables into an SFNT byte buffer per the OpenType spec: a 12-byte offset
/// table, a 16-byte directory entry per table (sorted by tag), then each table's data padded to
/// a 4-byte boundary. Per-table checksums are computed; the `head` table's `checkSumAdjustment`
/// is left as-is (font backends parse without validating it).
fn assemble_sfnt(sfnt_version: u32, tables: std::collections::HashMap<u32, Box<[u8]>>) -> Vec<u8> {
    let mut entries: Vec<(u32, Box<[u8]>)> = tables.into_iter().collect();
    entries.sort_by_key(|(tag, _)| *tag);
    let num_tables = entries.len() as u16;

    // Binary-search hint fields: largest power of two <= num_tables.
    let mut entry_selector = 0u16;
    while (1u16 << (entry_selector + 1)) <= num_tables {
        entry_selector += 1;
    }
    let search_range = (1u16 << entry_selector) * 16;
    let range_shift = num_tables.wrapping_mul(16).wrapping_sub(search_range);

    let mut directory = Vec::with_capacity(16 * entries.len());
    let mut data = Vec::new();
    let mut offset = 12 + 16 * entries.len();
    for (tag, table) in &entries {
        directory.extend_from_slice(&tag.to_be_bytes());
        directory.extend_from_slice(&sfnt_table_checksum(table).to_be_bytes());
        directory.extend_from_slice(&(offset as u32).to_be_bytes());
        directory.extend_from_slice(&(table.len() as u32).to_be_bytes());
        data.extend_from_slice(table);
        while data.len() % 4 != 0 {
            data.push(0);
        }
        offset += (table.len() + 3) & !3;
    }

    let mut out = Vec::with_capacity(12 + directory.len() + data.len());
    out.extend_from_slice(&sfnt_version.to_be_bytes());
    out.extend_from_slice(&num_tables.to_be_bytes());
    out.extend_from_slice(&search_range.to_be_bytes());
    out.extend_from_slice(&entry_selector.to_be_bytes());
    out.extend_from_slice(&range_shift.to_be_bytes());
    out.extend_from_slice(&directory);
    out.extend_from_slice(&data);
    out
}

/// SFNT table checksum: the sum of the table's contents read as big-endian `u32`s, with the
/// final partial word zero-padded, in wrapping (mod 2^32) arithmetic.
fn sfnt_table_checksum(data: &[u8]) -> u32 {
    let mut sum = 0u32;
    for chunk in data.chunks(4) {
        let mut word = [0u8; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        sum = sum.wrapping_add(u32::from_be_bytes(word));
    }
    sum
}

/// Parse a `unicode-range` hex bound, expanding `?` wildcards to `0` (low bound) or `F`
/// (high bound), e.g. `U+00??` → `0x0000..=0x00FF`.
fn parse_hex_bound(s: &str, high: bool) -> Option<u32> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let filled: String = s
        .chars()
        .map(|c| {
            if c == '?' {
                if high {
                    'F'
                } else {
                    '0'
                }
            } else {
                c
            }
        })
        .collect();
    u32::from_str_radix(&filled, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify `decode_web_font` turns a real WOFF2 payload into an SFNT the font stack can
    /// parse. Reads the fixture path from `GOSUB_WOFF2_FIXTURE` so we neither hit the network
    /// nor commit a binary font; skips when unset.
    #[test]
    fn decode_web_font_woff2_roundtrips_to_sfnt() {
        let Ok(path) = std::env::var("GOSUB_WOFF2_FIXTURE") else {
            eprintln!("skipping: set GOSUB_WOFF2_FIXTURE to a .woff2 file to run");
            return;
        };
        let woff2 = std::fs::read(&path).expect("read fixture");
        assert_eq!(&woff2[0..4], b"wOF2", "fixture must be WOFF2");

        let url = url::Url::parse("https://example.test/font.woff2").unwrap();
        let sfnt = decode_web_font(woff2, &url).expect("fixture within the size cap");

        // Output must be a different, valid SFNT (TrueType `0x00010000` or OpenType `OTTO`).
        let magic = u32::from_be_bytes([sfnt[0], sfnt[1], sfnt[2], sfnt[3]]);
        assert!(magic == 0x0001_0000 || magic == 0x4F54_544F, "not SFNT: {magic:#010x}");

        // It must re-parse and expose the core tables a backend reads.
        use allsorts::binary::read::ReadScope;
        use allsorts::font_data::FontData;
        use allsorts::tables::FontTableProvider;
        let font = ReadScope::new(&sfnt).read::<FontData<'_>>().expect("parse SFNT");
        let provider = font.table_provider(0).expect("table provider");
        for tag in [allsorts::tag::HEAD, allsorts::tag::CMAP, allsorts::tag::GLYF] {
            assert!(provider.has_table(tag), "missing table {tag:#010x}");
        }
    }

    /// A one-table (`cmap`) WOFF2 font whose header claims `total_sfnt_size` and whose
    /// compressed stream inflates to `table`.
    fn woff2_font(total_sfnt_size: u32, table: &[u8]) -> Vec<u8> {
        use std::io::Write as _;
        let mut encoder = brotli::CompressorWriter::new(Vec::new(), 4096, 11, 22);
        encoder.write_all(table).unwrap();
        let compressed = encoder.into_inner();

        // Flags 0 is `cmap`, untransformed; origLength 16 as a one-byte UIntBase128.
        let directory = [0x00u8, 16];
        let length = (48 + directory.len() + compressed.len()) as u32;
        let mut font = Vec::new();
        font.extend_from_slice(b"wOF2");
        font.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        font.extend_from_slice(&length.to_be_bytes());
        font.extend_from_slice(&1u16.to_be_bytes()); // numTables
        font.extend_from_slice(&0u16.to_be_bytes()); // reserved
        font.extend_from_slice(&total_sfnt_size.to_be_bytes());
        font.extend_from_slice(&(compressed.len() as u32).to_be_bytes());
        font.extend_from_slice(&[0; 4]); // major/minor version
        font.extend_from_slice(&[0; 20]); // metadata and private blocks: none
        font.extend_from_slice(&directory);
        font.extend_from_slice(&compressed);
        font
    }

    /// A one-table WOFF1 font header and directory; the table data itself is never read.
    fn woff_font(total_sfnt_size: u32, orig_length: u32) -> Vec<u8> {
        let mut font = Vec::new();
        font.extend_from_slice(b"wOFF");
        font.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        font.extend_from_slice(&64u32.to_be_bytes()); // length
        font.extend_from_slice(&1u16.to_be_bytes()); // numTables
        font.extend_from_slice(&0u16.to_be_bytes()); // reserved
        font.extend_from_slice(&total_sfnt_size.to_be_bytes());
        font.extend_from_slice(&[0; 24]); // versions, metadata and private blocks
        font.extend_from_slice(b"cmap");
        font.extend_from_slice(&64u32.to_be_bytes()); // offset
        font.extend_from_slice(&8u32.to_be_bytes()); // compLength
        font.extend_from_slice(&orig_length.to_be_bytes());
        font.extend_from_slice(&0u32.to_be_bytes()); // origChecksum
        font
    }

    /// A WOFF2 whose header admits to unpacking past the cap is refused before any inflate,
    /// and the face is dropped rather than handed to a backend.
    #[test]
    fn woff2_over_cap_total_sfnt_size_is_rejected() {
        let font = woff2_font(MAX_WEB_FONT_SIZE as u32 + 1, &[0; 16]);
        let err = check_woff2_size(&font, MAX_WEB_FONT_SIZE).unwrap_err();
        assert!(err.contains("totalSfntSize"), "{err}");

        let url = url::Url::parse("https://example.test/font.woff2").unwrap();
        assert!(decode_web_font(font, &url).is_none());
    }

    /// A WOFF2 that claims a tiny size but whose Brotli stream inflates past the cap is
    /// caught by the bounded inflate; the same stream under a roomier cap passes.
    #[test]
    fn woff2_brotli_bomb_is_rejected() {
        let font = woff2_font(64, &[0; 64 * 1024]);
        assert!(font.len() < 1024, "fixture should compress well: {} bytes", font.len());

        let err = check_woff2_size(&font, 4096).unwrap_err();
        assert!(err.contains("Brotli"), "{err}");
        assert!(check_woff2_size(&font, 64 * 1024).is_ok());
    }

    /// WOFF1 is passed through to the backends, so only its header bounds what they inflate:
    /// an over-cap `totalSfntSize` or table `origLength` drops the face.
    #[test]
    fn woff_over_cap_header_is_rejected() {
        let cap = MAX_WEB_FONT_SIZE as u32;
        let err = check_woff_size(&woff_font(cap + 1, 16), MAX_WEB_FONT_SIZE).unwrap_err();
        assert!(err.contains("totalSfntSize"), "{err}");
        let err = check_woff_size(&woff_font(64, cap + 1), MAX_WEB_FONT_SIZE).unwrap_err();
        assert!(err.contains("tables unpack"), "{err}");

        let url = url::Url::parse("https://example.test/font.woff").unwrap();
        assert!(decode_web_font(woff_font(cap + 1, 16), &url).is_none());
        let ok = woff_font(64, 16);
        assert_eq!(decode_web_font(ok.clone(), &url), Some(ok));
    }

    /// The SFNT rebuilt from a WOFF2 font is held to the cap too, since rebuilding can grow it
    /// past the Brotli stream it came from.
    #[test]
    fn a_rebuilt_sfnt_over_the_cap_is_rejected() {
        let cap = MAX_WEB_FONT_SIZE;
        assert!(check_sfnt_size(cap as usize, cap).is_ok());
        let err = check_sfnt_size(cap as usize + 1, cap).unwrap_err();
        assert!(err.contains("rebuilt font"), "{err}");
    }

    /// Each way a source can fail sends the face on to the next: nothing fetched, a font that
    /// does not decode (an over-cap WOFF), and a font the font system refuses. The face lands
    /// on the first source that gets through all three, and no later source is fetched.
    #[tokio::test]
    async fn every_failing_src_falls_through_to_the_next() {
        use gosub_interface::font::FontError;
        let url = |name: &str| Url::parse(&format!("https://fonts.test/{name}")).unwrap();
        let face = Face {
            family: "Test".into(),
            sources: vec![
                url("missing"),
                url("bomb.woff"),
                url("refused"),
                url("good"),
                url("spare"),
            ],
        };
        let fetched = Mutex::new(Vec::new());
        let fetch = |u: Url| {
            fetched.lock().push(u.path().to_string());
            let body = match u.path() {
                "/missing" => None,
                "/bomb.woff" => Some(woff_font(MAX_WEB_FONT_SIZE as u32 + 1, 16)),
                "/refused" => Some(b"refused".to_vec()),
                _ => Some(b"good".to_vec()),
            };
            async move { body }
        };
        let mut registered = Vec::new();
        let mut register = |bytes: Vec<u8>, family: &str| {
            if bytes == b"refused" {
                return Err(FontError::InvalidFont("no".into()));
            }
            registered.push((family.to_string(), bytes));
            Ok(())
        };

        let first = load_face(&face, 0, &fetch).await;
        assert!(register_face(&face, first, &fetch, &mut register).await);
        assert_eq!(registered, vec![("Test".to_string(), b"good".to_vec())]);
        assert_eq!(*fetched.lock(), vec!["/missing", "/bomb.woff", "/refused", "/good"]);
    }

    /// A face none of whose sources works registers nothing.
    #[tokio::test]
    async fn a_face_with_no_working_src_registers_nothing() {
        let face = Face {
            family: "Test".into(),
            sources: vec![Url::parse("https://fonts.test/a").unwrap()],
        };
        let fetch = |_: Url| async { None };
        let mut register =
            |_: Vec<u8>, _: &str| -> Result<(), gosub_interface::font::FontError> { panic!("nothing to register") };
        let first = load_face(&face, 0, &fetch).await;
        assert!(!register_face(&face, first, &fetch, &mut register).await);
    }
}
