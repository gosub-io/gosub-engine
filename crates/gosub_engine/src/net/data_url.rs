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
/// missing, or when the payload does not decode. A caller should treat that exactly as it
/// treats a failed fetch: the resource is not coming.
pub fn decode(url: &Url) -> Option<(String, Vec<u8>)> {
    if url.scheme() != "data" {
        return None;
    }

    // `Url` keeps everything after `data:` in the path, except that a payload containing a
    // `#` (legal in base64? no, but legal in percent-encoded text) would land in the
    // fragment. Rebuilding from the pieces keeps such a payload whole.
    let mut rest = url.path().to_string();
    if let Some(query) = url.query() {
        rest.push('?');
        rest.push_str(query);
    }
    if let Some(fragment) = url.fragment() {
        rest.push('#');
        rest.push_str(fragment);
    }

    let (meta, payload) = rest.split_once(',')?;
    let (media_type, is_base64) = match meta.strip_suffix(";base64") {
        Some(head) => (head, true),
        None => (meta, false),
    };
    let media_type = if media_type.is_empty() {
        DEFAULT_MEDIA_TYPE.to_string()
    } else {
        media_type.to_string()
    };

    let bytes = if is_base64 {
        // The URL parser has already removed any tabs and newlines an author wrapped a long
        // payload with, so this is belt and braces for a `Url` built by other means.
        let cleaned: String = payload.chars().filter(|c| !c.is_ascii_whitespace()).collect();
        base64::engine::general_purpose::STANDARD
            .decode(cleaned.as_bytes())
            .ok()?
    } else {
        percent_decode(payload)?
    };

    Some((media_type, bytes))
}

/// Percent-decoding for a non-base64 payload. Bytes are taken as they come: a `data:` URL
/// may name any charset, so this does not assume UTF-8.
fn percent_decode(input: &str) -> Option<Vec<u8>> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hex = bytes.get(i + 1..i + 3)?;
                let text = std::str::from_utf8(hex).ok()?;
                out.push(u8::from_str_radix(text, 16).ok()?);
                i += 3;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    Some(out)
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
    fn a_payload_containing_a_hash_survives_url_parsing() {
        // `Url` files everything after `#` as the fragment; a decoder reading only the path
        // would silently truncate the payload rather than fail, which is the worst outcome.
        let (_, bytes) = decode(&url("data:text/plain,before%20#after")).expect("decodes");
        assert_eq!(bytes, b"before #after");
    }
}
