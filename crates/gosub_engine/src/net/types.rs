pub use gosub_sonar::net::types::{
    BodyStream, FetchHandle, FetchKeyData, FetchRequest, FetchRequestBuilder, FetchResult, FetchResultMeta, NetError,
    Priority, RequestBody,
};

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
