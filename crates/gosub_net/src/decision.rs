//! Decision-making logic for handling fetched responses.
//! This includes MIME type sniffing and deciding whether to render, download, block, etc
//! based on the response metadata, content, request destination, and user-agent policy.

use crate::decision::sniff::{sniff_class, ResponseClass};
use crate::decision::types::{DecisionOutcome, HandlingDecision, RenderTarget, RequestDestination};
use crate::net_types::FetchResultMeta;
use crate::policy::UaPolicy;
use crate::types::PeekBuf;

mod sniff;
pub mod types;

/// Decide how the user agent should handle a fetched response.
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

    let mut effective_class = declared_mime.as_ref().and_then(class_from_mime).or(sniffed_class);

    if policy.enable_sniffing_navigation_upgrade
        && matches!(dest, RequestDestination::Document)
        && effective_class.is_none()
    {
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
        let looks_like_pdf = declared_mime.as_ref().map(|m| mime_is_pdf(m)).unwrap_or(false)
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
        ResponseClass::Html | ResponseClass::XHtml | ResponseClass::Xml => {
            HandlingDecision::Render(RenderTarget::HtmlParser)
        }
        ResponseClass::Image => HandlingDecision::Render(RenderTarget::ImageDecoder),
        ResponseClass::Js => HandlingDecision::Render(RenderTarget::JsEngine),
        ResponseClass::Css => HandlingDecision::Render(RenderTarget::CssParser),
        ResponseClass::Pdf => HandlingDecision::Download {
            path: std::path::PathBuf::new(),
        },
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
        .map_or(false, |s| s.eq_ignore_ascii_case(val_ci))
}

/// Check if the `Content-Disposition` header indicates an attachment.
fn content_disposition_is_attachment(meta: &FetchResultMeta) -> bool {
    meta.headers
        .get(http::header::CONTENT_DISPOSITION)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_ascii_lowercase().starts_with("attachment"))
        .unwrap_or(false)
}

/// Check if a MIME type is PDF.
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
    } else if m.type_() == mime::APPLICATION && m.subtype() == "octet-stream" {
        None
    } else {
        None
    }
}
