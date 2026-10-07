//! Opaque Response Blocking: which response bodies may enter a renderer.
//!
//! Process isolation puts a site's rendering in its own process so that a bug
//! (or a Spectre gadget) there can only read that process's memory. That is
//! worth something only if cross-origin secrets never get *into* that memory.
//! Pages load cross-origin subresources constantly - images, scripts,
//! stylesheets, fonts - so the network layer cannot refuse them; ORB is the line
//! it draws instead, decided here in the broker before bytes are handed on:
//!
//! - **Same-origin** bodies are the page's own.
//! - Cross-origin bodies of **embeddable** types (images, media, CSS, scripts,
//!   fonts) are delivered: the renderer can paint or run them, and the engine
//!   gives it no way to read them back as data.
//! - Cross-origin bodies that are **data** - HTML, JSON, XML - or that sniff as
//!   such never leave the broker. A cross-origin `secret.json` pulled in through
//!   `<img src>` is exactly how a compromised renderer would try to read another
//!   origin's data.
//!
//! The decision follows the shape of the WHATWG ORB algorithm: MIME type first,
//! then a sniff of the body's first bytes for the unknown cases, with an image
//! sniff as the escape hatch for mislabelled images (common in the wild). Types
//! ORB never sniffs - CSV, event streams, archives, PDF, office documents - are
//! blocked on the label alone: a sniff could not tell a logged-in user's CSV
//! export from harmless text.
//! There is no CORS input yet: every subresource the engine issues today is a
//! no-cors load; `fetch()` will add the CORS-approved branch.

/// What may happen to a cross-origin response body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrbVerdict {
    Allow,
    /// Withhold the body; the reason names what was recognised.
    Block(&'static str),
}

/// Bytes of the body the sniff looks at.
const SNIFF_LEN: usize = 1024;

/// Decide for one response. `same_origin` compares the requesting document's
/// origin with the response's final (post-redirect) URL; `content_type` is the
/// raw header value; `nosniff` is `X-Content-Type-Options: nosniff`; `body` is
/// the start of the body (only its first [`SNIFF_LEN`] bytes are examined).
pub fn verdict(same_origin: bool, content_type: Option<&str>, nosniff: bool, status: u16, body: &[u8]) -> OrbVerdict {
    if same_origin {
        return OrbVerdict::Allow;
    }
    let essence = content_type.map(essence_of).unwrap_or_default();
    let body = &body[..body.len().min(SNIFF_LEN)];

    // Before the safelist: `audio/mpegurl` is a playlist, not audio.
    if is_never_sniffed(&essence) {
        return OrbVerdict::Block("cross-origin type that is never sniffed");
    }
    if is_opaque_safelisted(&essence) {
        return OrbVerdict::Allow;
    }
    if is_blocklisted(&essence) {
        return OrbVerdict::Block("cross-origin data type");
    }
    if nosniff && essence == "text/plain" {
        return OrbVerdict::Block("cross-origin text/plain with nosniff");
    }
    if !(200..300).contains(&status) {
        return OrbVerdict::Block("cross-origin error response");
    }
    if sniffs_as_image(body) {
        return OrbVerdict::Allow;
    }
    if sniffs_as_html(body) {
        return OrbVerdict::Block("cross-origin body sniffs as HTML");
    }
    if sniffs_as_xml(body) {
        return OrbVerdict::Block("cross-origin body sniffs as XML");
    }
    if sniffs_as_json(body) {
        return OrbVerdict::Block("cross-origin body sniffs as JSON");
    }
    OrbVerdict::Allow
}

/// `type/subtype`, lowercased, parameters dropped.
fn essence_of(content_type: &str) -> String {
    use cow_utils::CowUtils;
    content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .cow_to_ascii_lowercase()
        .into_owned()
}

/// Types a page may embed cross-origin without CORS. `application/octet-stream`
/// is not one: it says nothing about the body, so it goes to the sniff like an
/// absent type (a font or image served as it still passes).
fn is_opaque_safelisted(essence: &str) -> bool {
    if essence.starts_with("image/")
        || essence.starts_with("video/")
        || essence.starts_with("audio/")
        || essence.starts_with("font/")
    {
        return true;
    }
    matches!(
        essence,
        "text/css"
            | "text/javascript"
            | "application/javascript"
            | "application/x-javascript"
            | "application/ecmascript"
            | "text/ecmascript"
            | "application/wasm"
            | "application/ogg"
            | "application/x-font-ttf"
            | "application/x-font-otf"
            | "application/x-font-woff"
            | "application/font-woff"
            | "application/font-woff2"
            | "application/vnd.ms-fontobject"
    )
}

/// ORB's opaque-blocklisted-never-sniffed types: data by declaration that no
/// sniff could recognise, so the label alone blocks them.
fn is_never_sniffed(essence: &str) -> bool {
    matches!(
        essence,
        "application/dash+xml"
            | "application/gzip"
            | "application/msexcel"
            | "application/mspowerpoint"
            | "application/msword"
            | "application/msword-template"
            | "application/pdf"
            | "application/vnd.apple.mpegurl"
            | "application/vnd.ces-quickpoint"
            | "application/vnd.ces-quicksheet"
            | "application/vnd.ces-quickword"
            | "application/vnd.ms-excel"
            | "application/vnd.ms-excel.sheet.macroenabled.12"
            | "application/vnd.ms-powerpoint"
            | "application/vnd.ms-powerpoint.presentation.macroenabled.12"
            | "application/vnd.ms-word"
            | "application/vnd.ms-word.document.12"
            | "application/vnd.ms-word.document.macroenabled.12"
            | "application/vnd.msword"
            | "application/vnd.openxmlformats-officedocument.presentationml.presentation"
            | "application/vnd.openxmlformats-officedocument.presentationml.template"
            | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
            | "application/vnd.openxmlformats-officedocument.spreadsheetml.template"
            | "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            | "application/vnd.openxmlformats-officedocument.wordprocessingml.template"
            | "application/vnd.presentation-openxml"
            | "application/vnd.presentation-openxmlm"
            | "application/vnd.spreadsheet-openxml"
            | "application/vnd.wordprocessing-openxml"
            | "application/x-gzip"
            | "application/x-protobuf"
            | "application/x-protobuffer"
            | "application/zip"
            | "audio/mpegurl"
            | "multipart/byteranges"
            | "multipart/signed"
            | "text/event-stream"
            | "text/csv"
            | "text/vtt"
    )
}

/// Types that are data by declaration: never delivered, whatever they sniff as.
fn is_blocklisted(essence: &str) -> bool {
    matches!(
        essence,
        "text/html" | "text/xml" | "application/xml" | "application/json" | "text/json"
    ) || essence.ends_with("+xml")
        || essence.ends_with("+json")
}

fn sniffs_as_image(body: &[u8]) -> bool {
    body.starts_with(b"\x89PNG\r\n\x1a\n")
        || body.starts_with(b"\xff\xd8\xff")
        || body.starts_with(b"GIF87a")
        || body.starts_with(b"GIF89a")
        || (body.len() >= 12 && &body[..4] == b"RIFF" && &body[8..12] == b"WEBP")
        || body.starts_with(b"BM")
        || body.starts_with(b"\x00\x00\x01\x00")
        || body.starts_with(b"<svg")
}

fn leading_text(body: &[u8]) -> &[u8] {
    let start = body
        .iter()
        .position(|b| !matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0c))
        .unwrap_or(body.len());
    let body = &body[start..];
    body.strip_prefix(b"\xef\xbb\xbf").unwrap_or(body)
}

fn sniffs_as_html(body: &[u8]) -> bool {
    let text = leading_text(body);
    if !text.starts_with(b"<") {
        return false;
    }
    let tag: String = text[1..]
        .iter()
        .take(12)
        .take_while(|b| b.is_ascii_alphabetic() || **b == b'!')
        .map(|b| b.to_ascii_lowercase() as char)
        .collect();
    matches!(
        tag.as_str(),
        "!doctype"
            | "html"
            | "head"
            | "body"
            | "script"
            | "iframe"
            | "title"
            | "meta"
            | "link"
            | "style"
            | "div"
            | "p"
            | "table"
            | "a"
            | "!--"
    )
}

fn sniffs_as_xml(body: &[u8]) -> bool {
    leading_text(body).starts_with(b"<?xml")
}

fn sniffs_as_json(body: &[u8]) -> bool {
    let text = leading_text(body);
    // The anti-hijacking prefix some APIs emit.
    if text.starts_with(b")]}'") {
        return true;
    }
    if !text.starts_with(b"{") && !text.starts_with(b"[") {
        return false;
    }
    // An opener alone is not JSON: untyped script can start with `[` or a
    // `{` block too. It is JSON if it parses, or if the parse only ran out of
    // input - the sniff sees at most SNIFF_LEN bytes of a longer body.
    match serde_json::from_slice::<serde_json::Value>(text) {
        Ok(_) => true,
        Err(error) => error.is_eof(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// JSON is blocked whether it fits the sniff or is cut off by it; script
    /// that merely starts with `[` or `{` is not mistaken for it.
    #[test]
    fn json_is_told_from_script_that_starts_like_it() {
        let block = |body: &[u8]| verdict(false, None, false, 200, body) != OrbVerdict::Allow;
        assert!(block(br#"{"user": "x", "token": "secret"}"#));
        assert!(block(b"[1, 2, 3]"));
        assert!(block(b")]}'\n{\"a\":1}"));
        // A JSON array longer than the sniff: truncated mid-value, still JSON.
        let long = format!("[{}]", vec!["\"padding-entry\""; 200].join(","));
        assert!(long.len() > SNIFF_LEN);
        assert!(block(long.as_bytes()));
        // Untyped script that happens to open with an array or a block.
        assert!(!block(b"[1, 2].forEach(function (n) { console.log(n); });"));
        assert!(!block(b"{ init(); }"));
    }

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR";

    #[test]
    fn same_origin_is_always_delivered() {
        for (ct, body) in [
            (Some("application/json"), b"{\"secret\":1}".as_slice()),
            (Some("text/html"), b"<!doctype html>".as_slice()),
            (None, b"anything".as_slice()),
        ] {
            assert_eq!(verdict(true, ct, false, 200, body), OrbVerdict::Allow);
        }
    }

    #[test]
    fn cross_origin_data_types_are_blocked() {
        for ct in [
            "text/html",
            "text/html; charset=utf-8",
            "application/json",
            "application/ld+json",
            "text/xml",
            "application/xml",
        ] {
            assert!(
                matches!(verdict(false, Some(ct), false, 200, b"..."), OrbVerdict::Block(_)),
                "{ct}"
            );
        }
    }

    #[test]
    fn cross_origin_embeddable_types_are_delivered() {
        for ct in [
            "image/png",
            "image/jpeg",
            "text/css",
            "application/javascript",
            "text/javascript; charset=utf-8",
            "font/woff2",
            "application/font-woff",
            "video/mp4",
            // SVG is an image: safelisted despite the +xml suffix.
            "image/svg+xml",
        ] {
            assert_eq!(
                verdict(false, Some(ct), false, 200, b"<!doctype html>"),
                OrbVerdict::Allow,
                "{ct}"
            );
        }
    }

    #[test]
    fn never_sniffed_types_are_blocked_on_the_label() {
        for ct in [
            "text/csv",
            "text/csv; charset=utf-8",
            "TEXT/CSV",
            "text/event-stream",
            "application/pdf",
            "application/zip",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "application/x-protobuf",
            "text/vtt",
            // A playlist, though the safelist takes `audio/*`.
            "audio/mpegurl",
        ] {
            // Even a body that sniffs as an image does not get it through.
            assert!(
                matches!(verdict(false, Some(ct), false, 200, PNG), OrbVerdict::Block(_)),
                "{ct}"
            );
        }
        // The page's own CSV export is its own business.
        assert_eq!(verdict(true, Some("text/csv"), false, 200, b"a,b"), OrbVerdict::Allow);
    }

    /// `application/octet-stream` is the usual label of a JSON or HTML
    /// "download" endpoint; it says nothing, so the body decides.
    #[test]
    fn octet_stream_is_sniffed() {
        let ct = Some("application/octet-stream");
        for body in [
            b"<!doctype html><html>".as_slice(),
            br#"{"token": "secret"}"#.as_slice(),
            b"<?xml version=\"1.0\"?>".as_slice(),
        ] {
            assert!(matches!(verdict(false, ct, false, 200, body), OrbVerdict::Block(_)));
        }
        assert_eq!(verdict(false, ct, false, 200, PNG), OrbVerdict::Allow);
        // A font served as octet-stream, as many CDNs do.
        assert_eq!(
            verdict(false, ct, false, 200, b"wOF2\x00\x01\x00\x00"),
            OrbVerdict::Allow
        );
        assert!(matches!(verdict(false, ct, false, 404, PNG), OrbVerdict::Block(_)));
    }

    #[test]
    fn unknown_types_are_sniffed() {
        // A mislabelled image is still an image.
        assert_eq!(verdict(false, Some("text/plain"), false, 200, PNG), OrbVerdict::Allow);
        assert_eq!(verdict(false, None, false, 200, PNG), OrbVerdict::Allow);
        // Data hiding behind an unknown or absent type is caught by its shape.
        for body in [
            b"<!DOCTYPE html><html>".as_slice(),
            b"\n\n  <html lang=en>".as_slice(),
            b"<?xml version=\"1.0\"?>".as_slice(),
            b" {\"token\": \"x\"}".as_slice(),
            b")]}'\n{}".as_slice(),
            b"[1,2,3]".as_slice(),
        ] {
            assert!(matches!(verdict(false, None, false, 200, body), OrbVerdict::Block(_)));
            assert!(matches!(
                verdict(false, Some("text/plain"), false, 200, body),
                OrbVerdict::Block(_)
            ));
        }
        // Plain text that is none of those passes.
        assert_eq!(
            verdict(false, Some("text/plain"), false, 200, b"hello world"),
            OrbVerdict::Allow
        );
        assert_eq!(verdict(false, None, false, 200, b"\x00\x01binary"), OrbVerdict::Allow);
    }

    #[test]
    fn nosniff_and_errors_are_blocked_cross_origin() {
        assert!(matches!(
            verdict(false, Some("text/plain"), true, 200, b"hello"),
            OrbVerdict::Block(_)
        ));
        assert!(matches!(
            verdict(false, None, false, 404, b"nope"),
            OrbVerdict::Block(_)
        ));
        // ...but a safelisted type survives both.
        assert_eq!(verdict(false, Some("image/png"), true, 404, b""), OrbVerdict::Allow);
    }
}
