pub use gosub_sonar::net::types::{
    BodyStream, FetchRequest, FetchRequestBuilder, FetchResult, FetchResultMeta, NetError, Priority, RequestBody,
};

/// Engine-side handle for an in-flight fetch.
///
/// gosub-sonar 0.2.0 removed its public `FetchHandle` (the request-coalescing key is
/// internal to the fetcher now; `Fetcher::submit` takes a bare `CancellationToken`).
/// The engine keeps its own handle carrying what its pipelines actually use: the
/// request id for bookkeeping and the token that cancels the fetch and its children.
#[derive(Debug, Clone)]
pub struct FetchHandle {
    pub req_id: gosub_sonar::types::RequestId,
    pub cancel: tokio_util::sync::CancellationToken,
}

/// What kind of resource is being fetched.
///
/// gosub-sonar only distinguishes coarse categories (`Primary`/`Asset`/`Other`), so the
/// engine keeps this richer classification for its own events and pipelines and maps it
/// down via [`ResourceKind::to_net`] when building a `FetchRequest`. The original rich
/// value is kept per request in the
/// [`REF_REGISTRY`](crate::net::req_ref_tracker::REF_REGISTRY) so fetcher callbacks can
/// recover it.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum ResourceKind {
    Document,
    Stylesheet,
    Script { blocking: bool },
    Image,
    Font,
    Media,
    Xhr,
    Fetch,
    WebSocket,
    Other,
}

impl ResourceKind {
    /// The `Accept` request-header value a browser sends for this resource kind.
    pub fn accept_header(self) -> &'static str {
        match self {
            ResourceKind::Document => {
                "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8"
            }
            ResourceKind::Stylesheet => "text/css,*/*;q=0.1",
            ResourceKind::Image => "image/avif,image/webp,image/apng,image/svg+xml,image/*,*/*;q=0.8",
            _ => "*/*",
        }
    }

    /// Map to the coarse classification the gosub-sonar fetcher understands.
    pub fn to_net(self) -> gosub_sonar::net::types::ResourceKind {
        match self {
            ResourceKind::Document => gosub_sonar::net::types::ResourceKind::Primary,
            ResourceKind::Other => gosub_sonar::net::types::ResourceKind::Other,
            _ => gosub_sonar::net::types::ResourceKind::Asset,
        }
    }

    /// Best-effort mapping back from the coarse net-side classification. Only used as a
    /// fallback when the rich value was not registered for the request.
    pub fn from_net(kind: gosub_sonar::net::types::ResourceKind) -> Self {
        match kind {
            gosub_sonar::net::types::ResourceKind::Primary => ResourceKind::Document,
            gosub_sonar::net::types::ResourceKind::Asset | gosub_sonar::net::types::ResourceKind::Other => {
                ResourceKind::Other
            }
        }
    }
}

/// Who or what triggered the fetch.
///
/// Same story as [`ResourceKind`]: richer than gosub-sonar's `User`/`Application`/`Other`,
/// mapped down at the fetch boundary.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum Initiator {
    /// Initiated by the user, UI, or link click
    Navigation,
    /// HTML Parser resource
    Parser,
    /// Initiated by a JS script (or Lua script) (fetch, XHR)
    Script,
    /// CSS @import, font-face
    CSS,
    /// Other undefined type of initiator
    Other,
}

impl Initiator {
    /// Map to the coarse classification the gosub-sonar fetcher understands.
    pub fn to_net(self) -> gosub_sonar::net::types::Initiator {
        match self {
            Initiator::Navigation => gosub_sonar::net::types::Initiator::User,
            Initiator::Parser | Initiator::Script | Initiator::CSS => gosub_sonar::net::types::Initiator::Application,
            Initiator::Other => gosub_sonar::net::types::Initiator::Other,
        }
    }

    /// Best-effort mapping back from the coarse net-side classification. Only used as a
    /// fallback when the rich value was not registered for the request.
    pub fn from_net(initiator: gosub_sonar::net::types::Initiator) -> Self {
        match initiator {
            gosub_sonar::net::types::Initiator::User => Initiator::Navigation,
            gosub_sonar::net::types::Initiator::Application => Initiator::Parser,
            gosub_sonar::net::types::Initiator::Other => Initiator::Other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use bytes::Bytes;

    use http::HeaderMap;
    use tokio::io::AsyncReadExt;
    use url::Url;

    fn dummy_meta() -> FetchResultMeta {
        FetchResultMeta {
            final_url: Url::parse("https://example.org/").unwrap(),
            status: 200,
            status_text: "OK".into(),
            headers: HeaderMap::new(),
            content_length: None,
            content_type: None,
            has_body: true,
        }
    }

    mod response_info {
        use super::super::ResponseInfo;
        use url::Url;

        fn info(url: &str, headers: &[(&str, &str)]) -> ResponseInfo {
            ResponseInfo {
                final_url: Url::parse(url).unwrap(),
                status: 200,
                status_text: "OK".into(),
                headers: headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
                content_length: None,
                content_type: None,
                has_body: true,
            }
        }

        #[test]
        fn header_lookup_ignores_case() {
            let i = info("https://example.org/", &[("Content-Disposition", "attachment")]);
            assert_eq!(i.header("content-disposition"), Some("attachment"));
            assert_eq!(i.header("CONTENT-DISPOSITION"), Some("attachment"));
            assert_eq!(i.header("content-type"), None);
        }

        #[test]
        fn attachment_detection_ignores_case() {
            assert!(info(
                "https://example.org/",
                &[("content-disposition", "ATTACHMENT; filename=\"a\"")]
            )
            .is_attachment());
            assert!(!info("https://example.org/", &[("content-disposition", "inline")]).is_attachment());
            assert!(!info("https://example.org/", &[]).is_attachment());
        }

        #[test]
        fn filename_prefers_disposition_then_url_then_fallback() {
            assert_eq!(
                info(
                    "https://example.org/x.bin",
                    &[("content-disposition", "attachment; filename=\"real.pdf\"")]
                )
                .suggested_filename(),
                "real.pdf"
            );
            assert_eq!(info("https://example.org/dir/x.bin", &[]).suggested_filename(), "x.bin");
            // No usable path segment and no disposition.
            assert_eq!(info("https://example.org/", &[]).suggested_filename(), "download");
        }

        #[test]
        fn filename_is_percent_decoded_from_the_url() {
            assert_eq!(
                info("https://example.org/my%20file%20(1).txt", &[]).suggested_filename(),
                "my file (1).txt"
            );
        }

        /// A hostile `Content-Disposition` must not escape the directory the embedder picked.
        #[test]
        fn filename_never_carries_path_separators() {
            for hostile in [
                "attachment; filename=\"../../etc/passwd\"",
                "attachment; filename=\"/etc/passwd\"",
                "attachment; filename=\"..\\\\windows\\\\system32\\\\evil.dll\"",
            ] {
                let name = info("https://example.org/x", &[("content-disposition", hostile)]).suggested_filename();
                assert!(!name.contains('/'), "{hostile} -> {name}");
                assert!(!name.contains('\\'), "{hostile} -> {name}");
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn bodystream_from_bytes_reads_all() {
        let data = Bytes::from_static(b"hello world");
        let mut s = BodyStream::from_bytes(data.clone());
        assert_eq!(s.len, Some(11));
        assert!(s.is_seekable);
        assert!(s.clonable);

        let mut out = Vec::new();
        s.read_to_end(&mut out).await.unwrap();
        assert_eq!(&out[..], &data[..]);

        let n = s.read(&mut [0u8; 8]).await.unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn fetchresult_debug_and_clone() {
        let meta = dummy_meta();
        let body = Bytes::from_static(b"DATA");
        let r1 = FetchResult::Buffered {
            meta: meta.clone(),
            body: body.clone(),
        };

        let dbg = format!("{r1:?}");
        assert!(dbg.contains("FetchResult::Buffered"));
        assert!(dbg.contains("body_len: 4"));
        assert!(dbg.contains("status: 200"));

        let r2 = r1.clone();
        match r2 {
            FetchResult::Buffered { meta: m, body: b } => {
                assert_eq!(m.status, 200);
                assert_eq!(&b[..], b"DATA");
            }
            _ => panic!("expected buffered"),
        }
    }
}

use cow_utils::CowUtils;

/// What the server said about a response, in the engine's own vocabulary.
///
/// Same story as [`ResourceKind`] and [`Initiator`]: the network layer's `FetchResultMeta`
/// comes from the external gosub-sonar crate, and putting it in a public event would pin the
/// engine's API to that crate's struct layout. This is the engine-owned shape, built from it
/// at the event boundary. Headers are a plain list rather than a `HeaderMap`, matching
/// [`ResourceEvent::Headers`](crate::events::ResourceEvent::Headers).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseInfo {
    /// Final URL, after any redirects.
    pub final_url: url::Url,
    /// HTTP status code.
    pub status: u16,
    /// HTTP status reason phrase.
    pub status_text: String,
    /// Response headers, in the order received. Names are as served; use
    /// [`header`](Self::header) for case-insensitive lookup.
    pub headers: Vec<(String, String)>,
    /// `Content-Length`, when the server gave one.
    pub content_length: Option<u64>,
    /// `Content-Type`, without parameters stripped.
    pub content_type: Option<String>,
    /// Whether a body follows (a `HEAD` response has none).
    pub has_body: bool,
}

impl ResponseInfo {
    /// Case-insensitive header lookup, returning the first match.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// Whether `Content-Disposition` marks this as an attachment - i.e. the server asked for
    /// it to be saved rather than displayed.
    pub fn is_attachment(&self) -> bool {
        self.header("content-disposition")
            .is_some_and(|v| v.cow_to_ascii_lowercase().contains("attachment"))
    }

    /// The filename to offer when saving: `Content-Disposition`'s `filename` when present,
    /// otherwise the URL's last path segment, otherwise `"download"`. Never a path - any
    /// directory components are stripped.
    pub fn suggested_filename(&self) -> String {
        let from_disposition = self.header("content-disposition").and_then(|v| {
            v.split(';').find_map(|part| {
                let part = part.trim();
                part.strip_prefix("filename=")
                    .map(|f| f.trim_matches('"').to_string())
                    .filter(|f| !f.is_empty())
            })
        });
        let name = from_disposition.or_else(|| {
            self.final_url
                .path_segments()
                .and_then(|mut segments| segments.next_back())
                .filter(|segment| !segment.is_empty())
                .map(|segment| {
                    percent_encoding::percent_decode_str(segment)
                        .decode_utf8_lossy()
                        .into_owned()
                })
        });
        let name = name.unwrap_or_default();
        let name = name.rsplit(['/', '\\']).next().unwrap_or("").trim().to_string();
        if name.is_empty() {
            "download".to_string()
        } else {
            name
        }
    }
}

impl From<&FetchResultMeta> for ResponseInfo {
    fn from(meta: &FetchResultMeta) -> Self {
        Self {
            final_url: meta.final_url.clone(),
            status: meta.status,
            status_text: meta.status_text.clone(),
            headers: meta
                .headers
                .iter()
                .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or_default().to_string()))
                .collect(),
            content_length: meta.content_length,
            content_type: meta.content_type.clone(),
            has_body: meta.has_body,
        }
    }
}
