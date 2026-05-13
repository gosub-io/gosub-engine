pub use gosub_net::net::types::{
    BodyStream, FetchHandle, FetchKeyData, FetchRequest, FetchResultMeta, Initiator, NetError,
    Priority, ResourceKind,
};

use crate::engine::types::PeekBuf;
use crate::html::DummyDocument;
use crate::net::shared_body::SharedBody;
use bytes::Bytes;
use std::fmt::Debug;
use std::path::PathBuf;
use std::sync::Arc;

/// FetchResult defines the resource response. Either a stream or buffered response are possible
#[derive(Clone)]
pub enum FetchResult {
    /// Streamed response body
    Stream {
        meta: FetchResultMeta,
        peek_buf: PeekBuf,
        shared: Arc<SharedBody>,
    },
    /// Buffered response body
    Buffered { meta: FetchResultMeta, body: Bytes },
    /// Network error occurred
    Error(NetError),
}

impl FetchResult {
    pub fn is_error(&self) -> bool {
        matches!(self, FetchResult::Error(_))
    }

    pub fn meta(&self) -> Option<&FetchResultMeta> {
        match self {
            FetchResult::Stream { meta, .. } => Some(meta),
            FetchResult::Buffered { meta, .. } => Some(meta),
            FetchResult::Error(_) => None,
        }
    }
}

impl Debug for FetchResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchResult::Stream { meta, .. } => f.debug_struct("FetchResult::Stream").field("meta", meta).finish(),
            FetchResult::Buffered { meta, body } => f
                .debug_struct("FetchResult::Buffered")
                .field("meta", meta)
                .field("body_len", &body.len())
                .finish(),
            FetchResult::Error(e) => f.debug_tuple("FetchResult::Error").field(e).finish(),
        }
    }
}

/// The outcome of a main-frame navigation.
#[derive(Debug)]
pub enum ObsoleteNavigationResult {
    Document {
        meta: FetchResultMeta,
        doc: DummyDocument,
    },
    DownloadStarted {
        meta: FetchResultMeta,
        dest: PathBuf,
    },
    DownloadFinished {
        meta: FetchResultMeta,
        dest: PathBuf,
    },
    OpenExternalStarted {
        meta: FetchResultMeta,
        dest: PathBuf,
    },
    Cancelled,
    Failed {
        meta: Option<FetchResultMeta>,
        error: Arc<anyhow::Error>,
    },
    RenderedByViewer {
        meta: FetchResultMeta,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::shared_body::SharedBody;
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

    #[test]
    fn fetchresult_stream_variant_compiles() {
        let meta = dummy_meta();
        let shared = Arc::new(SharedBody::new(8));
        let _r = FetchResult::Stream {
            meta,
            peek_buf: PeekBuf::from_slice(b"PEEK"),
            shared,
        };
    }
}
