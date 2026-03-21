use crate::types::PeekBuf;
use mime::Mime;
use mimetype_detector::detect;
use std::str::FromStr;

// Coarse response class used for routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseClass {
    Html,
    XHtml,
    Xml,
    Text,
    Css,
    Js,
    Json,
    Image,
    Audio,
    Video,
    Font,
    Pdf,
    Binary,
    Unknown,
}

impl ResponseClass {
    /// Map a MIME type to a ResponseClass.
    pub fn from_mime(m: &Mime) -> Self {
        let top = m.type_().as_str();
        let sub = m.subtype().as_str();

        match (top, sub) {
            ("text", "html") => ResponseClass::Html,
            ("application", "xhtml+xml") => ResponseClass::XHtml,
            ("application", "xml") | ("text", "xml") => ResponseClass::Xml,
            ("application", "json") | ("text", "json") => ResponseClass::Json,
            ("text", "plain") => ResponseClass::Text,
            ("text", "css") => ResponseClass::Css,
            ("application", "javascript") | ("text", "javascript") | ("application", "ecmascript") => ResponseClass::Js,

            ("image", _) => ResponseClass::Image,
            ("audio", _) => ResponseClass::Audio,
            ("video", _) => ResponseClass::Video,

            ("font", _) => ResponseClass::Font,
            ("application", "font-woff") => ResponseClass::Font,
            ("application", "font-woff2") => ResponseClass::Font,
            ("application", "font-ttf") => ResponseClass::Font,
            ("application", "vnd.ms-fontobject") => ResponseClass::Font,

            ("application", "pdf") => ResponseClass::Pdf,
            ("application", "octet-stream") => ResponseClass::Binary,

            _ => ResponseClass::Unknown,
        }
    }
}

/// Sniff the content type from the given peek buffer and return the corresponding ResponseClass.
pub fn sniff_class(peek_buf: PeekBuf) -> ResponseClass {
    let mime_type = detect(peek_buf.as_slice());
    let mime = Mime::from_str(format!("{}/{}", mime_type.mime(), mime_type.extension()).as_str())
        .unwrap_or(mime::APPLICATION_OCTET_STREAM);
    ResponseClass::from_mime(&mime)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mime::Mime;

    #[test]
    pub fn test_from_mime() {
        let cases = vec![
            ("text/html", ResponseClass::Html),
            ("application/xhtml+xml", ResponseClass::XHtml),
            ("text/plain", ResponseClass::Text),
            ("text/css", ResponseClass::Css),
            ("application/javascript", ResponseClass::Js),
            ("text/javascript", ResponseClass::Js),
            ("application/ecmascript", ResponseClass::Js),
            ("image/png", ResponseClass::Image),
            ("audio/mpeg", ResponseClass::Audio),
            ("video/mp4", ResponseClass::Video),
            ("font/woff", ResponseClass::Font),
            ("application/font-woff", ResponseClass::Font),
            ("application/pdf", ResponseClass::Pdf),
            ("application/octet-stream", ResponseClass::Binary),
            ("application/unknown", ResponseClass::Unknown),
        ];

        for (mime_str, expected_class) in cases {
            let mime: Mime = mime_str.parse().unwrap();
            let class = ResponseClass::from_mime(&mime);
            assert_eq!(class, expected_class, "MIME: {}", mime_str);
        }
    }

    #[test]
    pub fn test_sniff_class() {
        let html_peek =
            PeekBuf::from_slice(b"<!DOCTYPE html><html><head><title>Test</title></head><body></body></html>");
        let css_peek = PeekBuf::from_slice(b"body { background-color: #fff; }");
        let js_peek = PeekBuf::from_slice(b"console.log('Hello, world!');");
        let png_peek = PeekBuf::from_slice(b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01");
        let mp3_peek = PeekBuf::from_slice(b"ID3\x03\x00\x00\x00\x00\x0fTIT2\x00\x00\x00\x0f\x00\x00Test Title");
        let woff_peek = PeekBuf::from_slice(b"\x77\x4F\x46\x46"); // 'wOFF'
        let pdf_peek =
            PeekBuf::from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        let unknown_peek = PeekBuf::from_slice(b"\x00\x01\x02\x03\x04");

        assert_eq!(sniff_class(html_peek), ResponseClass::Html);
        assert_eq!(sniff_class(css_peek), ResponseClass::Css);
        assert_eq!(sniff_class(js_peek), ResponseClass::Js);
        assert_eq!(sniff_class(png_peek), ResponseClass::Image);
        assert_eq!(sniff_class(mp3_peek), ResponseClass::Audio);
        assert_eq!(sniff_class(woff_peek), ResponseClass::Font);
        assert_eq!(sniff_class(pdf_peek), ResponseClass::Pdf);
        assert_eq!(sniff_class(unknown_peek), ResponseClass::Binary);
    }
}
