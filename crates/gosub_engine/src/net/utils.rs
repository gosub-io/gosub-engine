use crate::engine::types::PeekBuf;
use crate::net::types::{FetchResult, NetError};
use crate::net::SharedBody;
use bytes::Bytes;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::pin::Pin;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, ReadBuf};
use tokio::sync::{oneshot, Mutex};
use url::Url;

/// An entry in the waiter, representing a listener and whether it wants streaming or buffered response.
struct WaiterEntry {
    /// Listener for this entry.
    tx: oneshot::Sender<FetchResult>,
    /// Whether the listener wants a streaming response (true) or buffered (false).
    wants_streaming: bool,
}

// Simple waiter for coalescing responses. If a fetcher detects we are requesting the same resource
// that is already queued, we add them to the waiter for that request, so the request will fetch the
// resource only once and dispatches the result to all listeners. Will also handle the case where some
// listeners want streaming results, and some want buffered results.
#[derive(Default)]
pub struct Waiter {
    /// List of listeners (oneshot senders) waiting for the result.
    listeners: Mutex<Vec<WaiterEntry>>,
}

impl Waiter {
    pub fn new() -> Self {
        Self {
            listeners: Mutex::new(Vec::new()),
        }
    }

    #[cfg(test)]
    pub(crate) fn new_arc() -> Arc<Waiter> {
        Arc::new(Waiter::new())
    }

    /// Register a consumer for this waiter. We need to know if the consumer is streaming or not.
    pub async fn register(&self, tx: oneshot::Sender<FetchResult>, wants_streaming: bool) {
        self.listeners.lock().await.push(WaiterEntry { tx, wants_streaming })
    }

    /// Process the fetch result with the listeners.
    pub async fn finish(self: &Arc<Self>, result: FetchResult) {
        let mut ls = self.listeners.lock().await;

        match result {
            FetchResult::Buffered { meta, body } => {
                // Buffered results can be sent to all listeners as-is
                let res = FetchResult::Buffered {
                    meta: meta.clone(),
                    body: body.clone(),
                };
                for entry in ls.drain(..) {
                    let _ = entry.tx.send(res.clone());
                }
            }
            FetchResult::Stream { meta, peek_buf, shared } => {
                // Streamed results need to be fanned out to streaming listeners, but buffered listeners
                // need to have the stream read to the end and buffered first.
                let mut streaming_ls = Vec::new();
                let mut buffered_ls = Vec::new();
                while let Some(entry) = ls.pop() {
                    if entry.wants_streaming {
                        streaming_ls.push(entry.tx);
                    } else {
                        buffered_ls.push(entry.tx);
                    }
                }

                // Send the stream to all the streaming listeners
                for tx in streaming_ls {
                    let res = FetchResult::Stream {
                        meta: meta.clone(),
                        peek_buf: peek_buf.clone(),
                        shared: shared.clone(),
                    };
                    let _ = tx.send(res);
                }

                // Send the stream as buffered to all the buffered listeners
                if !buffered_ls.is_empty() {
                    // This will read the stream to the end, so it might take some time.
                    match stream_to_bytes(peek_buf, shared).await {
                        Ok(b) => {
                            let res = FetchResult::Buffered {
                                meta: meta.clone(),
                                body: b,
                            };
                            for tx in buffered_ls {
                                let _ = tx.send(res.clone());
                            }
                        }
                        Err(e) => {
                            let res = FetchResult::Error(NetError::Read(Arc::new(e)));
                            for tx in buffered_ls {
                                let _ = tx.send(res.clone());
                            }
                        }
                    }
                }
            }
            FetchResult::Error(e) => {
                let res = FetchResult::Error(e.clone());
                for entry in ls.drain(..) {
                    let _ = entry.tx.send(res.clone());
                }
            }
        }
    }
}

/// Convert a streaming body a buffered fetch-result by reading it to the end.
/// This could be more efficient with allocations, probably.
pub async fn stream_to_bytes(peek_buf: PeekBuf, shared: Arc<SharedBody>) -> anyhow::Result<Bytes> {
    // Allocate for at least peek buffer, plus some additional to start the streaming
    let mut out = Vec::with_capacity(peek_buf.len() + 8192);

    let mut reader = SharedBody::combined_reader(peek_buf, shared);
    if let Err(e) = reader.read_to_end(&mut out).await {
        return Err(NetError::Io(Arc::new(e)).into());
    }

    Ok(Bytes::from(out))
}

/// Normalizes a URL by removing its fragment and returning it as a string.
pub fn normalize_url(u: &Url) -> String {
    let mut u = u.clone();
    u.set_fragment(None);
    u.as_str().to_string()
}

/// Computes a short hash for a given byte slice.
pub fn short_hash(bytes: &[u8]) -> u64 {
    let mut h = DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

/// Create a short url (with ... truncated)
pub fn short_url(u: &Url, max: usize) -> String {
    let s = u.as_str();
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

// Small async reader for BodyStream::from_bytes
pub struct BytesAsyncReader {
    pub data: Bytes,
    pub pos: usize,
}

impl tokio::io::AsyncRead for BytesAsyncReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let remaining = self.data.len().saturating_sub(self.pos);
        if remaining == 0 {
            return std::task::Poll::Ready(Ok(())); // EOF
        }
        let to_copy = std::cmp::min(remaining, buf.remaining());
        let end = self.pos + to_copy;
        buf.put_slice(&self.data[self.pos..end]);
        self.pos = end;
        std::task::Poll::Ready(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::shared_body::SharedBody;
    use crate::net::types::FetchResultMeta;
    use tokio::io::AsyncReadExt;
    use tokio::time::{sleep, Duration};

    fn dummy_meta() -> FetchResultMeta {
        FetchResultMeta {
            final_url: Url::parse("https://example.org/").unwrap(),
            status: 200,
            status_text: "OK".into(),
            headers: http::HeaderMap::new(),
            content_length: None,
            content_type: None,
            has_body: true,
        }
    }

    #[test]
    fn normalize_url_strips_fragment() {
        let u = Url::parse("https://example.org/a/b#frag").unwrap();
        assert_eq!(normalize_url(&u), "https://example.org/a/b");
    }

    #[test]
    fn short_hash_differs_for_diff_inputs() {
        let a = short_hash(b"abc");
        let b = short_hash(b"abd");
        assert_ne!(a, b);
    }

    #[test]
    fn short_url_truncates() {
        let u = Url::parse("https://example.org/very/long/path/here").unwrap();
        let s = short_url(&u, 16);
        assert!(s.ends_with("..."));
        assert!(s.len() <= 19); // 16 + "..."
    }

    #[tokio::test(flavor = "current_thread")]
    async fn bytes_async_reader_reads_all() {
        let data = Bytes::from_static(b"hello world");
        let mut r = BytesAsyncReader { data, pos: 0 };
        let mut out = Vec::new();
        r.read_to_end(&mut out).await.unwrap();
        assert_eq!(&out[..], b"hello world");
        // second read should be EOF (0 bytes)
        let n = r.read(&mut [0u8; 8]).await.unwrap();
        assert_eq!(n, 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn waiter_finishes_buffered_to_all() {
        let waiter = Waiter::new_arc();
        let (tx1, rx1) = oneshot::channel();
        let (tx2, rx2) = oneshot::channel();
        waiter.register(tx1, false).await; // buffered
        waiter.register(tx2, true).await; // streaming-flag shouldn't matter for Buffered result

        let body = Bytes::from_static(b"BODY");
        let meta = dummy_meta();
        waiter
            .finish(FetchResult::Buffered {
                meta: meta.clone(),
                body: body.clone(),
            })
            .await;

        let r1 = rx1.await.unwrap();
        let r2 = rx2.await.unwrap();
        match r1 {
            FetchResult::Buffered { meta: m, body: b } => {
                assert_eq!(m.status, 200);
                assert_eq!(&b[..], b"BODY");
            }
            _ => panic!("expected buffered"),
        }
        match r2 {
            FetchResult::Buffered { meta: m, body: b } => {
                assert_eq!(m.status, 200);
                assert_eq!(&b[..], b"BODY");
            }
            _ => panic!("expected buffered"),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn waiter_stream_is_fanned_out_and_buffered_followers_convert() {
        let (tx_stream, rx_stream) = oneshot::channel();
        let (tx_buf, rx_buf) = oneshot::channel();

        let waiter = Waiter::new_arc();
        waiter.register(tx_stream, true).await; // wants streaming
        waiter.register(tx_buf, false).await; // wants buffered

        // Prepare shared body that will receive tail bytes shortly
        let shared = Arc::new(SharedBody::new(8));
        let shared_writer = shared.clone();

        // Push tail asynchronously after a short delay
        tokio::spawn(async move {
            // small delay makes sure finish() is waiting on stream_to_bytes
            sleep(Duration::from_millis(10)).await;
            shared_writer.push(Bytes::from_static(b"TAIL1"));
            shared_writer.push(Bytes::from_static(b"TAIL2"));
            shared_writer.finish();
        });

        let meta = dummy_meta();
        let peek_buf = PeekBuf::from_slice(b"PEEK-");

        // Finish with a Stream result (leader would produce this)
        waiter
            .finish(FetchResult::Stream {
                meta: meta.clone(),
                peek_buf: peek_buf.clone(),
                shared: shared.clone(),
            })
            .await;

        // Streaming listener should get Stream
        let r_stream = rx_stream.await.unwrap();
        match r_stream {
            FetchResult::Stream {
                meta: m, peek_buf: p, ..
            } => {
                assert_eq!(m.status, 200);
                assert_eq!(&p[..], b"PEEK-");
            }
            _ => panic!("expected stream"),
        }

        // Buffered listener should get the concatenation of peek + tail
        let r_buf = rx_buf.await.unwrap();
        match r_buf {
            FetchResult::Buffered { meta: m, body } => {
                assert_eq!(m.status, 200);
                assert_eq!(&body[..], b"PEEK-TAIL1TAIL2");
            }
            _ => panic!("expected buffered"),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn waiter_propagates_error() {
        let waiter = Waiter::new_arc();
        let (tx1, rx1) = oneshot::channel();
        let (tx2, rx2) = oneshot::channel();
        waiter.register(tx1, false).await;
        waiter.register(tx2, true).await;

        waiter
            .finish(FetchResult::Error(NetError::Cancelled("boom".into())))
            .await;

        let r1 = rx1.await.unwrap();
        let r2 = rx2.await.unwrap();
        matches!(r1, FetchResult::Error(_));
        matches!(r2, FetchResult::Error(_));
    }
}
