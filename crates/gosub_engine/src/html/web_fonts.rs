//! `@font-face` web fonts: extraction, fetching, decoding, registration.

use crate::html::RenderConfiguration;
use gosub_interface::font::FontError;
use gosub_interface::resource_loader::ResourceLoader;
use url::Url;

/// How a fetched face reaches a font system: `(bytes, css_family)`. A closure
/// rather than `&mut dyn FontSystem`, because the two callers hold their font
/// systems behind different locks.
pub(crate) type RegisterFont<'a> = &'a mut dyn FnMut(Vec<u8>, &str) -> Result<(), FontError>;

/// Fetch and register any `@font-face` web fonts declared in the document's
/// stylesheets. Deduplicates by resolved font URL; fetches are synchronous
/// through `loader`, so the caller decides where the blocking happens (the tab
/// worker runs this walk on a blocking thread; in a forked renderer, blocking
/// on the broker is the intended shape). Each face is handed to `register`
/// under its CSS family so the font system selects the right weight/style from
/// the font's own metadata.
pub(crate) fn load_web_fonts<C: RenderConfiguration>(
    doc: &C::Document,
    base_url: &Url,
    loader: &dyn ResourceLoader,
    register: RegisterFont<'_>,
) {
    use gosub_interface::css3::CssStylesheet as _;
    use gosub_interface::document::Document as _;

    let mut fetched: std::collections::HashSet<String> = std::collections::HashSet::new();
    for sheet in doc.stylesheets() {
        let sheet_url = Url::parse(sheet.url()).ok();
        for (family, sources, unicode_range) in sheet.font_faces() {
            // Google-style web fonts split a family into many `unicode-range` subsets
            // (latin, cyrillic, greek, …). We don't do per-glyph subset fallback, so
            // register only subsets covering Basic Latin (and ranges with no descriptor),
            // which covers Latin-script content without piling unusable subsets onto the
            // same family.
            if let Some(range) = &unicode_range {
                if !unicode_range_covers_basic_latin(range) {
                    continue;
                }
            }
            for src in &sources {
                let resolved = sheet_url
                    .as_ref()
                    .unwrap_or(base_url)
                    .join(src)
                    .or_else(|_| base_url.join(src));
                let Ok(font_url) = resolved else { continue };
                if !fetched.insert(font_url.to_string()) {
                    break; // this exact font file is already registered
                }
                match loader.load(&font_url) {
                    Ok(resp) if resp.is_ok() && !resp.body.is_empty() => {
                        // Web fonts are commonly served as WOFF2 (e.g. Google Fonts content-
                        // negotiates WOFF2 for modern UAs like ours). The font backends
                        // (Skia/fontconfig) only decode raw SFNT (TTF/OTF), so unwrap WOFF2
                        // to TTF first. Other formats pass through unchanged.
                        let font_bytes = decode_web_font(resp.body.to_vec(), &font_url);
                        match register(font_bytes, &family) {
                            Ok(()) => {
                                log::debug!("Registered web font '{family}' from {font_url}");
                                break; // family face loaded; skip remaining sources
                            }
                            Err(e) => log::warn!("Failed to register web font '{family}': {e:?}"),
                        }
                    }
                    Ok(resp) => log::warn!("Web font fetch {font_url} returned status {}", resp.status),
                    Err(e) => log::warn!("Web font fetch {font_url} failed: {e}"),
                }
            }
        }
    }
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

/// Unwrap a downloaded web-font payload into raw SFNT bytes the font backends can decode.
fn decode_web_font(bytes: Vec<u8>, font_url: &Url) -> Vec<u8> {
    const WOFF2_MAGIC: &[u8; 4] = b"wOF2";
    if bytes.len() < 4 || &bytes[0..4] != WOFF2_MAGIC {
        return bytes;
    }
    match woff2_to_sfnt(&bytes) {
        Ok(sfnt) => {
            log::debug!(
                "Decoded WOFF2 web font from {font_url} ({} → {} bytes)",
                bytes.len(),
                sfnt.len()
            );
            sfnt
        }
        Err(e) => {
            log::warn!("Failed to decode WOFF2 web font from {font_url}: {e}");
            bytes
        }
    }
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
        let sfnt = super::decode_web_font(woff2, &url);

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
}
