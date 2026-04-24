//! Decision-making logic for handling fetched responses.
//! This includes MIME type sniffing and deciding whether to render, download, block, etc
//! based on the response metadata, content, request destination, and user-agent policy.
//! This is a simplified version and does not yet implement all the detailed logic.

use crate::engine::types::PeekBuf;
use crate::engine::UaPolicy;
use crate::net::decision::sniff::{sniff_class, ResponseClass};
use crate::net::decision::types::{DecisionOutcome, HandlingDecision, RenderTarget, RequestDestination};
use crate::net::types::FetchResultMeta;
use cow_utils::CowUtils;

mod sniff;
pub mod types;

/// Decide how the user agent should handle a fetched response.
///
/// The decision is based on (in order):
/// 1) **UA policy gates** (feature flags like `enable_sniffing`, `enable_pdf_viewer`, etc.)
/// 2) **Header signals** (declared `Content-Type`, `X-Content-Type-Options: nosniff`,
///    `Content-Disposition` attachment/filename, etc.)
/// 3) **Sniffed class** from a peek buffer (when allowed)
///
/// High-level rules:
/// - If `Content-Disposition` indicates *attachment* and UA policy allows downloads without
///   user activation, prefer **Download**.
/// - If a trustworthy declared type is present, prefer it. If not trustworthy or absent,
///   and sniffing is enabled (and not blocked by `nosniff`), use the **sniffed** class.
/// - If the result is (or looks like) **PDF** and the UA has an embedded PDF viewer,
///   prefer **Render(PdfViewer)**.
/// - If navigation sniffing upgrade is enabled, allow HTML upgrade for mislabelled
///   navigations (e.g., `text/plain` / `application/octet-stream` that sniff as HTML).
///
/// Returns a [`DecisionOutcome`] with both the *final* class and the auxiliary evidence
/// (declared MIME, sniffed class, disposition flag).
pub fn decide_handling(
    // The metadata from the request, including headers like content-type, no-sniff etc.
    meta: &FetchResultMeta,
    // The request destination (e.g. "document", "script", "image", etc.)
    dest: RequestDestination,
    // A peek buffer containing the first few bytes of the response body.
    peek_buf: PeekBuf,
    // The user-agent policy, including settings like no-sniff, etc.
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
    let mut effective_class = declared_mime.as_ref().and_then(class_from_mime).or(sniffed_class);

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

    // If we are an attachment, and policy allows downloads without user activation, download.
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

    // Special case: if we look like a PDF, and the UA has a PDF viewer, render in it.
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

    // Final fallback: if we have no idea, treat as binary.
    let class = effective_class.unwrap_or(ResponseClass::Binary);

    let decision = match class {
        ResponseClass::Html | ResponseClass::XHtml | ResponseClass::Xml => {
            HandlingDecision::Render(RenderTarget::HtmlParser)
        }
        ResponseClass::Image => HandlingDecision::Render(RenderTarget::ImageDecoder),
        ResponseClass::Js => HandlingDecision::Render(RenderTarget::JsEngine),
        ResponseClass::Css => HandlingDecision::Render(RenderTarget::CssParser),
        ResponseClass::Pdf => {
            // If we reached here without pdf viewer enabled, download by default.
            HandlingDecision::Download {
                path: std::path::PathBuf::new(),
            }
        }
        ResponseClass::Json | ResponseClass::Text | ResponseClass::Binary => match dest {
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
        _ => {
            // Unknown or unhandled types: download by default.
            HandlingDecision::Download {
                path: std::path::PathBuf::new(),
            }
        }
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
    } else if mime_is_pdf(m) {
        Some(Pdf)
    } else if m.type_() == mime::APPLICATION && m.subtype() == "json" {
        Some(Json)
    } else if m.type_() == mime::TEXT {
        Some(Text)
    } else {
        None // treat as untrusted; let sniffing or policy decide
    }
}
