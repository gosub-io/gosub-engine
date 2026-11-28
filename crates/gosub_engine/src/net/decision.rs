//! Decision-making logic for handling fetched responses.
//! This includes MIME type sniffing and deciding whether to render, download, block, etc
//! based on the response metadata, content, request destination, and user-agent policy.

use crate::engine::types::PeekBuf;
use crate::engine::UaPolicy;
use crate::net::decision::sniff::sniff_class;
use crate::net::decision::types::{DecisionOutcome, HandlingDecision, RenderTarget, RequestDestination};
use crate::net::types::FetchResultMeta;
use cow_utils::CowUtils;

mod sniff;
pub(crate) use sniff::{sniff_mime, ResponseClass};
pub mod types;

/// Decide how the user agent should handle a fetched response, weighing UA
/// policy gates, header signals (`Content-Type`, `X-Content-Type-Options:
/// nosniff`, `Content-Disposition`) and, when allowed, a sniffed class from
/// the peek buffer. Returns the final class plus the evidence it came from.
pub fn decide_handling(
    meta: &FetchResultMeta,
    dest: RequestDestination,
    peek_buf: PeekBuf,
    policy: &UaPolicy,
) -> DecisionOutcome {
    let declared_mime = content_type_from_headers(meta);
    let nosniff = header_eq(meta, http::header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    let is_attachment = content_disposition_is_attachment(meta);

    let sniffed_class = if policy.enable_sniffing && !nosniff {
        Some(sniff_class(peek_buf))
    } else {
        None
    };

    // Prefer trustworthy declared types, otherwise fall back to sniffed type.
    let declared_class = declared_mime.as_ref().and_then(class_from_mime);
    let mut effective_class = declared_class.or(sniffed_class);

    // Optional "navigation sniffing upgrade": allow HTML if mislabelled during document nav.
    if policy.enable_sniffing_navigation_upgrade
        && matches!(dest, RequestDestination::Document)
        && effective_class.is_none()
    {
        // Common mislabels: text/plain or application/octet-stream that actually contain HTML.
        if let Some(sniff) = sniffed_class {
            if matches!(sniff, ResponseClass::Html | ResponseClass::Xml) {
                effective_class = Some(sniff);
            }
        }
    }

    if is_attachment && policy.allow_download_without_user_activation {
        return DecisionOutcome {
            class: effective_class.unwrap_or(ResponseClass::Binary),
            sniffed_class,
            declared_mime,
            disposition_attachment: true,
            decision: HandlingDecision::Download {
                path: std::path::PathBuf::new(),
            },
        };
    }

    if policy.enable_pdf_viewer {
        let looks_like_pdf = declared_mime.as_ref().map(mime_is_pdf).unwrap_or(false)
            || matches!(sniffed_class, Some(ResponseClass::Pdf));

        if looks_like_pdf {
            return DecisionOutcome {
                class: ResponseClass::Pdf,
                sniffed_class,
                declared_mime,
                disposition_attachment: false,
                decision: HandlingDecision::Render(RenderTarget::PdfViewer),
            };
        }
    }

    let class = effective_class.unwrap_or(ResponseClass::Binary);

    let decision = match class {
        // A declared XHTML attachment goes to the download offer. A text/html attachment
        // still renders here, which is wrong: an attachment is never rendered.
        ResponseClass::XHtml if is_attachment && declared_class == Some(ResponseClass::XHtml) => {
            HandlingDecision::Download {
                path: std::path::PathBuf::new(),
            }
        }
        // XHTML should go to an XML parser (HTML "Loading XML documents"). There is none yet,
        // so it is parsed as HTML; a self-closing `<script/>` or `<title/>` then swallows the
        // rest of the page. The HTML parser also reads neither the charset parameter nor the
        // XML declaration, so XHTML in another encoding than UTF-8 shows garbled unless it
        // has a byte order mark or a `<meta charset>`.
        ResponseClass::Html | ResponseClass::XHtml => HandlingDecision::Render(RenderTarget::HtmlParser),
        // There is no XML parser yet: a declared XML type is handled like text, which a
        // navigation shows as plain text. A sniffed one still goes to the HTML parser.
        ResponseClass::Xml if declared_class != Some(ResponseClass::Xml) => {
            HandlingDecision::Render(RenderTarget::HtmlParser)
        }
        ResponseClass::Image => HandlingDecision::Render(RenderTarget::ImageDecoder),
        ResponseClass::Js => HandlingDecision::Render(RenderTarget::JsEngine),
        ResponseClass::Css => HandlingDecision::Render(RenderTarget::CssParser),
        ResponseClass::Font => HandlingDecision::Render(RenderTarget::FontLoader),
        ResponseClass::Pdf => {
            // If we reached here without pdf viewer enabled, download by default.
            HandlingDecision::Download {
                path: std::path::PathBuf::new(),
            }
        }
        ResponseClass::Json | ResponseClass::Text | ResponseClass::Xml | ResponseClass::Binary => match dest {
            RequestDestination::Document
                if policy.enable_sniffing_navigation_upgrade
                    && matches!(sniffed_class, Some(ResponseClass::Html | ResponseClass::Xml)) =>
            {
                HandlingDecision::Render(RenderTarget::HtmlParser)
            }
            _ => HandlingDecision::Download {
                path: std::path::PathBuf::new(),
            },
        },
        _ => HandlingDecision::Download {
            path: std::path::PathBuf::new(),
        },
    };

    DecisionOutcome {
        class,
        sniffed_class,
        declared_mime,
        disposition_attachment: is_attachment,
        decision,
    }
}

/// Extract and parse the `Content-Type` header from the response metadata.
fn content_type_from_headers(meta: &FetchResultMeta) -> Option<mime::Mime> {
    meta.headers
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<mime::Mime>().ok())
}

/// Check if a header equals a given value, case-insensitively.
fn header_eq(meta: &FetchResultMeta, name: http::header::HeaderName, val_ci: &str) -> bool {
    meta.headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|s| s.eq_ignore_ascii_case(val_ci))
}

/// Check if the `Content-Disposition` header indicates an attachment.
fn content_disposition_is_attachment(meta: &FetchResultMeta) -> bool {
    meta.headers
        .get(http::header::CONTENT_DISPOSITION)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().cow_to_ascii_lowercase().starts_with("attachment"))
        .unwrap_or(false)
}

/// Check if a MIME type is PDF, case-insensitively.
fn mime_is_pdf(m: &mime::Mime) -> bool {
    (m.type_() == mime::APPLICATION && m.subtype() == "pdf") || m.essence_str().eq_ignore_ascii_case("application/pdf")
}

/// Map a MIME type to a coarse `ResponseClass`.
fn class_from_mime(m: &mime::Mime) -> Option<ResponseClass> {
    use ResponseClass::*;
    if m.type_() == mime::TEXT && m.subtype() == mime::HTML {
        Some(Html)
    } else if m.type_() == mime::TEXT && m.subtype() == mime::CSS {
        Some(Css)
    } else if m.type_() == mime::APPLICATION && m.subtype() == "javascript" {
        Some(Js)
    } else if m.type_() == mime::IMAGE {
        Some(Image)
    } else if m.type_() == mime::FONT {
        Some(Font)
    } else if mime_is_pdf(m) {
        Some(Pdf)
    } else if m.type_() == mime::APPLICATION && m.subtype() == "json" {
        Some(Json)
    } else if m.type_() == mime::APPLICATION && m.subtype() == "xhtml" && m.suffix() == Some(mime::XML) {
        Some(XHtml)
    } else if ((m.type_() == mime::TEXT || m.type_() == mime::APPLICATION)
        && m.subtype() == mime::XML
        && m.suffix().is_none())
        || m.suffix() == Some(mime::XML)
    {
        // An XML MIME type (MIME Sniffing, "XML MIME type"); image/svg+xml is an image above.
        Some(Xml)
    } else if m.type_() == mime::TEXT {
        Some(Text)
    } else {
        None // treat as untrusted; let sniffing or policy decide
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The policy the tab worker navigates with: no sniffing, no upgrade, no PDF viewer.
    fn navigation_policy() -> UaPolicy {
        UaPolicy {
            enable_sniffing: false,
            enable_sniffing_navigation_upgrade: false,
            enable_pdf_viewer: false,
            allow_download_without_user_activation: false,
        }
    }

    fn decide(headers: &[(&str, &str)], body: &[u8], dest: RequestDestination, policy: &UaPolicy) -> DecisionOutcome {
        let mut meta = FetchResultMeta::synthetic(url::Url::parse("https://example.test/doc").unwrap());
        for (name, value) in headers {
            meta.headers.insert(
                http::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
        }
        decide_handling(&meta, dest, PeekBuf::from_slice(body), policy)
    }

    fn navigate(content_type: &str) -> DecisionOutcome {
        decide(
            &[("content-type", content_type)],
            b"",
            RequestDestination::Document,
            &navigation_policy(),
        )
    }

    fn is_download(decision: &HandlingDecision) -> bool {
        matches!(decision, HandlingDecision::Download { .. })
    }

    #[test]
    fn an_xhtml_navigation_goes_to_the_html_parser() {
        for content_type in [
            "application/xhtml+xml",
            "application/xhtml+xml; charset=utf-8",
            "APPLICATION/XHTML+XML",
        ] {
            let outcome = navigate(content_type);
            assert_eq!(outcome.class, ResponseClass::XHtml, "{content_type}");
            assert_eq!(
                outcome.decision,
                HandlingDecision::Render(RenderTarget::HtmlParser),
                "{content_type}"
            );
        }
    }

    /// The router shows a non-attachment `Download` of class Xml as text.
    #[test]
    fn an_xml_navigation_is_classed_as_xml() {
        for content_type in [
            "application/xml",
            "text/xml",
            "application/rss+xml",
            "application/atom+xml; charset=utf-8",
        ] {
            let outcome = navigate(content_type);
            assert_eq!(outcome.class, ResponseClass::Xml, "{content_type}");
            assert!(is_download(&outcome.decision), "{content_type}: {:?}", outcome.decision);
        }
    }

    /// XML is text/xml, application/xml and `+xml` types, nothing else that says xml.
    #[test]
    fn another_xml_subtype_is_not_xml() {
        for content_type in ["audio/xml", "x-foo/xml", "application/xml+zip"] {
            assert_ne!(navigate(content_type).class, ResponseClass::Xml, "{content_type}");
        }
    }

    #[test]
    fn an_svg_navigation_is_still_an_image() {
        let outcome = navigate("image/svg+xml");
        assert_eq!(outcome.class, ResponseClass::Image);
        assert_eq!(outcome.decision, HandlingDecision::Render(RenderTarget::ImageDecoder));
    }

    #[test]
    fn an_xhtml_or_xml_attachment_is_downloaded() {
        for content_type in ["application/xhtml+xml", "application/xml"] {
            let outcome = decide(
                &[("content-type", content_type), ("content-disposition", "attachment")],
                b"",
                RequestDestination::Document,
                &navigation_policy(),
            );
            assert!(outcome.disposition_attachment, "{content_type}");
            assert!(is_download(&outcome.decision), "{content_type}: {:?}", outcome.decision);
        }
    }

    /// With sniffing on, XML that sniffs as XML still reaches the HTML parser, declared or not.
    #[test]
    fn sniffed_xml_still_goes_to_the_html_parser() {
        let policy = UaPolicy::default();
        for content_type in ["application/xml", "text/plain"] {
            let outcome = decide(
                &[("content-type", content_type)],
                b"<?xml version=\"1.0\"?><r/>",
                RequestDestination::Document,
                &policy,
            );
            assert_eq!(outcome.sniffed_class, Some(ResponseClass::Xml), "{content_type}");
            assert_eq!(
                outcome.decision,
                HandlingDecision::Render(RenderTarget::HtmlParser),
                "{content_type}"
            );
        }
    }

    #[test]
    fn nosniff_xml_is_classed_as_xml() {
        let outcome = decide(
            &[
                ("content-type", "application/xml"),
                ("x-content-type-options", "nosniff"),
            ],
            b"<?xml version=\"1.0\"?><r/>",
            RequestDestination::Document,
            &UaPolicy::default(),
        );
        assert_eq!(outcome.sniffed_class, None);
        assert_eq!(outcome.class, ResponseClass::Xml);
        assert!(is_download(&outcome.decision), "{:?}", outcome.decision);
    }

    #[test]
    fn an_xml_stylesheet_is_not_rendered() {
        let outcome = decide(
            &[("content-type", "application/xml")],
            b"",
            RequestDestination::Style,
            &navigation_policy(),
        );
        assert!(is_download(&outcome.decision), "{:?}", outcome.decision);
    }
}
