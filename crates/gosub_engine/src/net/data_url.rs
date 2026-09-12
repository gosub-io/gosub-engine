//! `data:` URLs, decoded where they are asked for rather than fetched.
//!
//! A `data:` URL carries its own bytes, so there is nothing to request: no connection, no
//! cache, no referrer, no policy to apply. That makes it the odd one out in a network stack
//! built around submitting a request and waiting, and the reason it is handled here instead
//! of in the fetcher: a consumer that hits one decodes it and is done.
//!
//! The syntax is RFC 2397: `data:[<mediatype>][;base64],<data>`. An absent media type means
//! `text/plain;charset=US-ASCII`, and without `;base64` the payload is percent-encoded.

use crate::net::types::{FetchRequest, FetchResult, FetchResultMeta, NetError};
use base64::Engine as _;
use bytes::Bytes;
use gosub_sonar::net::events::NetEvent;
use gosub_sonar::net::observer::NetObserver;
use http::HeaderMap;
use std::sync::Arc;
use url::Url;

/// The default per RFC 2397, for a URL that names no media type.
const DEFAULT_MEDIA_TYPE: &str = "text/plain;charset=US-ASCII";

/// What a media type of bare parameters is understood to be a parameter *of*, so that
/// `data:;charset=utf-8,x` names `text/plain;charset=utf-8` rather than the nonsense type
/// `;charset=utf-8`. RFC 2397 spells the shorthand out explicitly.
const IMPLIED_MEDIA_TYPE: &str = "text/plain";

/// Base64 as a `data:` URL is allowed to write it: padding optional rather than required, and
/// the leftover bits of an unpadded final chunk dropped rather than rejected. This is the
/// forgiving-base64 decode of the Infra standard, which is what the data URL processor calls
/// for - a stricter engine turns URLs every other browser accepts into failed resources.
const BASE64: base64::engine::general_purpose::GeneralPurpose = base64::engine::general_purpose::GeneralPurpose::new(
    &base64::alphabet::STANDARD,
    base64::engine::GeneralPurposeConfig::new()
        .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent)
        .with_decode_allow_trailing_bits(true),
);

/// Whether this request is for the engine-served `data:` scheme.
///
/// gosub-sonar only speaks http(s), so the I/O thread answers these itself, the same way it
/// answers `file://`. Without this a `data:` stylesheet, script or font reaches the zone
/// fetcher and fails as an unsupported scheme - only images worked, because the media source
/// decodes them before ever submitting a request.
pub fn handles(req: &FetchRequest) -> bool {
    req.url.scheme() == "data"
}

/// Answer a `data:` request from the URL itself.
///
/// No policy gate, unlike `file://`: the bytes travel inside the URL, so serving one reads
/// neither the disk nor the network and crosses no privilege boundary. The worst a malformed
/// one can do is fail to decode.
pub async fn serve(req: &FetchRequest, observer: Arc<dyn NetObserver + Send + Sync>) -> FetchResult {
    let url = req.url.clone();
    observer.on_event(NetEvent::Started { url: url.clone() });
    let started = std::time::Instant::now();

    let Some((content_type, body)) = decode(&url) else {
        let err = NetError::Io(Arc::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "malformed data: URL",
        )));
        observer.on_event(NetEvent::Failed {
            url,
            error: anyhow::anyhow!(err.clone()),
        });
        return FetchResult::Error(err);
    };

    let body = Bytes::from(body);
    let len = body.len() as u64;
    let mut headers = HeaderMap::new();
    if let Ok(v) = content_type.parse() {
        headers.insert(http::header::CONTENT_TYPE, v);
    }
    if let Ok(v) = len.to_string().parse() {
        headers.insert(http::header::CONTENT_LENGTH, v);
    }
    observer.on_event(NetEvent::ResponseHeaders {
        url: url.clone(),
        status: 200,
        headers: headers.clone(),
    });
    observer.on_event(NetEvent::Progress {
        received_bytes: len,
        expected_length: Some(len),
        elapsed: started.elapsed(),
    });
    observer.on_event(NetEvent::Finished {
        received_bytes: len,
        elapsed: started.elapsed(),
        url: url.clone(),
    });

    FetchResult::Buffered {
        meta: {
            let mut meta = FetchResultMeta::synthetic(url);
            meta.headers = headers;
            meta.content_length = Some(len);
            meta.content_type = Some(content_type);
            meta.has_body = len > 0;
            meta
        },
        body,
    }
}

/// Decode a `data:` URL into its media type and bytes.
///
/// `None` when this is not a `data:` URL, when the comma separating metadata from payload is
/// missing, or when a base64 payload does not decode. A caller should treat that exactly as it
/// treats a failed fetch: the resource is not coming.
pub fn decode(url: &Url) -> Option<(String, Vec<u8>)> {
    if url.scheme() != "data" {
        return None;
    }

    // The payload is the URL serialized with its *fragment excluded*, so a `#` ends the data:
    // `data:text/plain,a#b` carries `a`. `Url` has already filed the fragment separately, so
    // excluding it is a matter of not putting it back. The query does serialize, and so is put
    // back: `data:text/plain,a?b` carries `a?b`.
    let mut rest = url.path().to_string();
    if let Some(query) = url.query() {
        rest.push('?');
        rest.push_str(query);
    }

    let (meta, payload) = rest.split_once(',')?;
    let meta = meta.trim();

    // Percent-decoding comes first and applies to the whole payload, base64 or not. That is
    // what makes `data:text/plain;base64,SGVsbG8%3D` - an author or a serializer escaping the
    // padding - decode rather than fail.
    let body = percent_decode(payload);

    // `;base64` is a parameter name, and parameter names are case-insensitive: `;BASE64` marks
    // the payload just as well.
    let (media_type, bytes) = match meta.rsplit_once(';') {
        Some((head, param)) if param.trim().eq_ignore_ascii_case("base64") => {
            let cleaned: Vec<u8> = body.into_iter().filter(|b| !b.is_ascii_whitespace()).collect();
            (head.trim_end(), BASE64.decode(cleaned).ok()?)
        }
        _ => (meta, body),
    };

    let media_type = if media_type.is_empty() {
        DEFAULT_MEDIA_TYPE.to_string()
    } else if media_type.starts_with(';') {
        format!("{IMPLIED_MEDIA_TYPE}{media_type}")
    } else {
        media_type.to_string()
    };

    Some((media_type, bytes))
}

/// Percent-decoding for the payload. Bytes are taken as they come: a `data:` URL may name any
/// charset, so this does not assume UTF-8.
///
/// A `%` that does not introduce two hex digits stands for itself instead of failing the
/// decode, per the URL standard - `data:text/plain,100%` is a readable resource, not a
/// malformed one.
fn percent_decode(input: &str) -> Vec<u8> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let escape = (bytes[i] == b'%')
            .then(|| bytes.get(i + 1..i + 3))
            .flatten()
            .filter(|hex| hex.iter().all(u8::is_ascii_hexdigit))
            .and_then(|hex| u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok());
        match escape {
            Some(b) => {
                out.push(b);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use gosub_sonar::net::null_emitter::NullEmitter;
    use gosub_sonar::types::RequestId;
    use http::Method;

    fn url(s: &str) -> Url {
        Url::parse(s).expect("valid url")
    }

    fn request(url: &Url) -> FetchRequest {
        FetchRequest::builder(Method::GET, url.clone())
            .with_req_id(RequestId::new())
            .build()
    }

    #[test]
    fn the_scheme_is_claimed_and_only_this_scheme() {
        assert!(handles(&request(&url("data:text/plain,x"))));
        assert!(!handles(&request(&url("https://example.test/a.css"))));
        assert!(!handles(&request(&url("file:///tmp/a.css"))));
    }

    #[tokio::test]
    async fn a_request_is_answered_from_the_url_itself() {
        // The path every non-image subresource takes: a stylesheet reaching the I/O thread as
        // an ordinary fetch. Before the scheme was served here it fell through to the zone
        // fetcher, which speaks only http(s), and failed.
        let result = serve(
            &request(&url("data:text/css,body%7Bcolor%3Ared%7D")),
            Arc::new(NullEmitter),
        )
        .await;
        match result {
            FetchResult::Buffered { meta, body } => {
                assert_eq!(meta.status, 200);
                assert_eq!(meta.content_type.as_deref(), Some("text/css"));
                assert_eq!(meta.content_length, Some(body.len() as u64));
                assert!(meta.has_body);
                assert_eq!(std::str::from_utf8(&body).unwrap(), "body{color:red}");
            }
            other => panic!("expected buffered response, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_malformed_url_fails_rather_than_serving_empty_bytes() {
        let result = serve(&request(&url("data:text/css;base64,!!!!")), Arc::new(NullEmitter)).await;
        assert!(
            matches!(result, FetchResult::Error(_)),
            "a payload that will not decode must not read as an empty stylesheet"
        );
    }

    #[test]
    fn base64_payloads_decode_with_their_media_type() {
        let (media, bytes) = decode(&url("data:image/png;base64,aGVsbG8=")).expect("decodes");
        assert_eq!(media, "image/png");
        assert_eq!(bytes, b"hello");
    }

    #[test]
    fn a_payload_wrapped_across_lines_still_decodes() {
        // What an author does to a long data URL in an HTML attribute. The URL parser strips
        // the newline before we ever see it, which is worth pinning: it is the reason this
        // decoder does not have to care.
        let (_, bytes) = decode(&url("data:image/png;base64,aGVs\nbG8=")).expect("decodes");
        assert_eq!(bytes, b"hello");
    }

    #[test]
    fn a_missing_media_type_gets_the_spec_default() {
        let (media, bytes) = decode(&url("data:,plain")).expect("decodes");
        assert_eq!(media, DEFAULT_MEDIA_TYPE);
        assert_eq!(bytes, b"plain");
    }

    #[test]
    fn percent_encoded_payloads_decode() {
        let (media, bytes) = decode(&url("data:text/plain,a%20b%2Fc")).expect("decodes");
        assert_eq!(media, "text/plain");
        assert_eq!(bytes, b"a b/c");
    }

    #[test]
    fn other_schemes_and_malformed_payloads_are_refused() {
        assert!(decode(&url("https://example.test/a.png")).is_none());
        // No comma: nothing separates the metadata from the payload.
        assert!(decode(&url("data:image/png;base64")).is_none());
        // Not base64.
        assert!(decode(&url("data:image/png;base64,!!!!")).is_none());
    }

    #[test]
    fn a_fragment_ends_the_payload() {
        // The data URL processor serializes the URL with the fragment excluded, so `#` ends
        // the data rather than being part of it - the same as it would be in any other URL.
        let (_, bytes) = decode(&url("data:text/plain,before%20#after")).expect("decodes");
        assert_eq!(bytes, b"before ");
    }

    #[test]
    fn a_query_is_part_of_the_payload() {
        // The other half of the fragment rule: `?` does serialize, so it is data.
        let (_, bytes) = decode(&url("data:text/plain,a?b=c")).expect("decodes");
        assert_eq!(bytes, b"a?b=c");
    }

    #[test]
    fn a_percent_encoded_base64_payload_decodes() {
        // Percent-decoding runs over the payload first, base64 or not, so an escaped `=` is
        // still padding by the time the base64 decoder sees it.
        let (_, bytes) = decode(&url("data:text/plain;base64,SGVsbG8%3D")).expect("decodes");
        assert_eq!(bytes, b"Hello");
    }

    #[test]
    fn the_base64_marker_is_case_insensitive() {
        let (media, bytes) = decode(&url("data:text/plain;BASE64,aGVsbG8=")).expect("decodes");
        assert_eq!(media, "text/plain");
        assert_eq!(bytes, b"hello");
    }

    #[test]
    fn unpadded_base64_decodes() {
        // Forgiving-base64: padding is optional, and the bits left over from the final chunk
        // are dropped rather than rejected.
        let (_, bytes) = decode(&url("data:text/plain;base64,SGVsbG8")).expect("decodes");
        assert_eq!(bytes, b"Hello");
    }

    #[test]
    fn a_media_type_of_bare_parameters_gets_the_implied_type() {
        // RFC 2397 shorthand: the charset alone stands for a parameter on `text/plain`, and a
        // consumer parsing `;charset=utf-8` as a media type would get nothing usable.
        let (media, bytes) = decode(&url("data:;charset=utf-8,hello")).expect("decodes");
        assert_eq!(media, "text/plain;charset=utf-8");
        assert_eq!(bytes, b"hello");
    }

    #[test]
    fn a_stray_percent_stands_for_itself() {
        // Percent-decoding does not fail on an escape that is not one; treating this URL as
        // malformed would drop a perfectly readable resource.
        let (_, bytes) = decode(&url("data:text/plain,100%")).expect("decodes");
        assert_eq!(bytes, b"100%");
    }
}
