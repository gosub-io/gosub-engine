//! Bounded, fan-out body stream with per-subscriber queues and drop-on-lag policy.
//!
//! `SharedBody` lets one producer push `Bytes` while any number of subscribers
//! receive them as a stream. Each subscriber has a bounded MPSC queue to cap
//! memory usage. If a subscriber can't keep up, we drop that subscriber rather
//! than stalling the producer (and other subscribers).
//!
//! Semantics:
//! - `push(Bytes)`: best-effort, non-blocking; slow subscribers are removed.
//! - `finish()`: closes all subscribers → they observe EOF (`None`) cleanly.
//! - `error(NetError)`: broadcasts an error to all, then closes.
//! - `subscribe_stream()`: returns a `Stream<Item = Result<Bytes, NetError>>`
//!   that starts receiving future chunks from this point onward.
//! - `combined_reader(peek, shared)`: convenient `AsyncRead` of `peek` then tail.

use crate::net_types::NetError;
use crate::types::PeekBuf;
use bytes::Bytes;
use futures_core::stream::BoxStream;
use futures_core::Stream;
use futures_util::{stream, StreamExt, TryStreamExt};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::io::StreamReader;
use tokio_util::sync::CancellationToken;

/// Bounded, fan-out byte stream with per-subscriber queues and drop-on-lag.
///
/// `SharedBody` lets one producer push `Bytes` while any number of subscribers
/// receive them as a `Stream<Item = Result<bytes::Bytes, NetError>>`.
///
/// - Each subscriber has its **own bounded queue** (capacity set on creation).
/// - If a subscriber can't keep up and its queue fills, it is **dropped**
///   (non-blocking broadcast; other subscribers keep receiving).
/// - `finish()` ends all subscribers with EOF; `error(e)` delivers `Err(e)`
///   and then ends.
///
/// Subscribers see **only future chunks** from the moment they subscribe
/// (no replay). Useful to tee a response body to multiple consumers such as
/// the HTML parser, a download writer, and a progress UI.
#[derive(Clone)]
pub struct SharedBody {
    inner: Arc<Mutex<State>>,
}

// Internal state of a shared body
struct State {
    /// Active subscribers
    subs: HashMap<u64, mpsc::Sender<Result<Bytes, NetError>>>,
    /// Monotic id for subscribers
    next_id: AtomicU64,
    /// Limit on how many subscribers per queue are allowed
    max_queue: usize,
    /// If true, any additional push() is ignored. The stream is closed.
    closed: bool,
}

impl SharedBody {
    /// Creates a new `SharedBody` with the given per-subscriber queue capacity.
    pub fn new(max_queue: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(State {
                subs: HashMap::new(),
                next_id: AtomicU64::new(1),
                max_queue,
                closed: false,
            })),
        }
    }

    /// Pushes a chunk to all current subscribers (best-effort, non-blocking).
    pub fn push(&self, chunk: Bytes) {
        let (subs, mut to_remove) = {
            let st = self.inner.lock().unwrap();
            if st.closed {
                return;
            }
            let subs: Vec<(u64, mpsc::Sender<_>)> = st.subs.iter().map(|(id, tx)| (*id, tx.clone())).collect();
            (subs, Vec::new())
        };

        // Try to send to each subscriber without blocking
        for (id, tx) in subs {
            match tx.try_send(Ok(chunk.clone())) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(_)) => {
                    // This subscriber is too slow; drop it
                    to_remove.push(id);
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    // This subscriber is gone; remove it
                    to_remove.push(id);
                }
            }
        }

        // Remove any subscribers that are placed on the remove list
        if !to_remove.is_empty() {
            let mut st = self.inner.lock().unwrap();
            for id in &to_remove {
                st.subs.remove(id);
            }
        }
    }

    /// Broadcasts an error to all subscribers and closes the stream.
    pub fn error(&self, e: NetError) {
        let senders: Vec<mpsc::Sender<Result<Bytes, NetError>>> = {
            let mut st = self.inner.lock().unwrap();
            if st.closed {
                return;
            }
            st.closed = true;
            st.subs.drain().map(|(_, tx)| tx).collect()
        };

        for tx in senders {
            let _ = tx.try_send(Err(e.clone()));
        }
    }

    /// Finishes the stream cleanly (EOF).
    pub fn finish(&self) {
        let _dropped: Vec<mpsc::Sender<Result<Bytes, NetError>>> = {
            let mut st = self.inner.lock().unwrap();
            if st.closed {
                return;
            }
            st.closed = true;
            st.subs.drain().map(|(_, tx)| tx).collect()
        };

        // dropping senders is enough; receivers yield None (EOF)
    }

    /// Subscribes with the given capacity, returning a stream of body chunks.
    pub fn subscribe_with_cap(&self, max_queue: usize) -> BoxStream<'static, Result<Bytes, NetError>> {
        let (maybe_rx, id_opt) = {
            let mut st = self.inner.lock().unwrap();
            if st.closed {
                return stream::empty::<Result<Bytes, NetError>>().boxed();
            }

            let (tx, rx) = mpsc::channel(max_queue);
            let id = st.next_id.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            st.subs.insert(id, tx);
            (Some(rx), Some(id))
        };

        let rx = maybe_rx.unwrap();
        let id = id_opt.unwrap();

        SubStream {
            id,
            parent: self.inner.clone(),
            inner: ReceiverStream::new(rx),
        }
        .boxed()
    }

    /// Subscribes with the default per-subscriber queue capacity.
    pub fn subscribe_stream(&self) -> BoxStream<'static, Result<Bytes, NetError>> {
        let cap = {
            let st = self.inner.lock().unwrap();
            st.max_queue
        };

        self.subscribe_with_cap(cap)
    }

    /// Returns an `AsyncRead` that yields `peek` first, then the live tail bytes.
    pub fn combined_reader(peek_buf: PeekBuf, shared: Arc<SharedBody>) -> Pin<Box<dyn AsyncRead + Send>> {
        let head = stream::iter([Ok::<Bytes, std::io::Error>(peek_buf.into_bytes())]);
        let rest_stream = shared.subscribe_stream().map_err(|e: NetError| e.to_io());

        let combined = head.chain(rest_stream);
        Box::pin(StreamReader::new(combined))
    }
}

/// Options to wrap an `AsyncRead` into a `SharedBody` via `SharedBody::from_reader`.
pub struct ReaderOptions {
    pub capacity: usize,
    pub buf_size: usize,
    pub cancel: Option<CancellationToken>,
    pub idle_timeout: Option<Duration>,
    pub total_timeout: Option<Duration>,
    pub max_size: Option<u64>,
}

impl Default for ReaderOptions {
    fn default() -> Self {
        Self {
            capacity: 32,
            buf_size: 16 * 1024,
            cancel: None,
            idle_timeout: None,
            total_timeout: None,
            max_size: None,
        }
    }
}

impl SharedBody {
    /// Spawns a background task that reads from `reader` and pushes chunks into
    /// a new `SharedBody`.
    pub fn from_reader<R>(mut reader: R, opts: ReaderOptions) -> Arc<Self>
    where
        R: AsyncRead + Send + 'static + Unpin,
    {
        let sb = Arc::new(SharedBody::new(opts.capacity));
        let sb_clone = sb.clone();

        tokio::spawn(async move {
            let ReaderOptions {
                capacity: _,
                buf_size,
                cancel,
                idle_timeout,
                total_timeout,
                max_size,
            } = opts;

            let deadline = total_timeout.map(|d| Instant::now() + d);
            let cancel = cancel.unwrap_or_else(CancellationToken::new);
            let mut buf = vec![0u8; buf_size];
            let mut total_read: u64 = 0;

            let check_total_deadline = |now: Instant| -> Result<(), NetError> {
                if let Some(dl) = deadline {
                    if now >= dl {
                        return Err(NetError::Timeout("total read timeout".to_string()));
                    }
                }
                Ok(())
            };

            if let Err(e) = check_total_deadline(Instant::now()) {
                sb_clone.error(e);
                return;
            }

            loop {
                if cancel.is_cancelled() {
                    sb_clone.error(NetError::Cancelled("read cancelled".to_string()));
                    return;
                }

                let read_cap = if let Some(max) = max_size {
                    let remaining = max.saturating_sub(total_read);
                    if remaining == 0 {
                        sb_clone.error(NetError::Io(Arc::new(std::io::Error::other(
                            "max size reached before read",
                        ))));
                    }
                    remaining.min(buf.len() as u64) as usize
                } else {
                    buf.len()
                };

                let read_res = if let Some(idle) = idle_timeout {
                    timeout(idle, reader.read(&mut buf[..read_cap]))
                        .await
                        .map_err(|_| NetError::Timeout("read idle timeout".to_string()))
                        .and_then(|r| r.map_err(|e| NetError::Io(Arc::new(e))))
                } else {
                    reader
                        .read(&mut buf[..read_cap])
                        .await
                        .map_err(|e| NetError::Io(Arc::new(e)))
                };

                match read_res {
                    Ok(0) => {
                        sb_clone.finish();
                        return;
                    }
                    Ok(n) => {
                        total_read = total_read.saturating_add(n as u64);
                        sb_clone.push(Bytes::copy_from_slice(&buf[..n]));

                        if let Some(max) = max_size {
                            if total_read > max {
                                sb_clone.error(NetError::Io(Arc::new(std::io::Error::other(
                                    "max size exceeded during read",
                                ))));
                                return;
                            }
                        }

                        if let Err(e) = check_total_deadline(Instant::now()) {
                            sb_clone.error(e);
                            return;
                        }
                    }
                    Err(e) => {
                        sb_clone.error(e);
                        return;
                    }
                }
            }
        });

        sb
    }
}

/// Per-subscriber stream returned by `SharedBody::subscribe_*`.
struct SubStream {
    id: u64,
    parent: Arc<Mutex<State>>,
    inner: ReceiverStream<Result<Bytes, NetError>>,
}

impl Stream for SubStream {
    type Item = Result<Bytes, NetError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        Pin::new(&mut this.inner).poll_next(cx)
    }
}

impl Drop for SubStream {
    fn drop(&mut self) {
        if let Ok(mut st) = self.parent.lock() {
            st.subs.remove(&self.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    #[tokio::test(flavor = "current_thread")]
    async fn shared_body_broadcasts_and_finishes() {
        let sb = SharedBody::new(8);

        let mut s1 = sb.subscribe_stream();
        let mut s2 = sb.subscribe_stream();

        sb.push(Bytes::from_static(b"hello"));
        sb.push(Bytes::from_static(b" world"));
        sb.finish();

        let a1 = s1.next().await.unwrap().unwrap();
        let a2 = s1.next().await.unwrap().unwrap();
        let expected1: &[u8] = b"hello";
        let expected2: &[u8] = b" world";
        assert_eq!((&a1[..], &a2[..]), (expected1, expected2));
        assert!(s1.next().await.is_none());

        let b1 = s2.next().await.unwrap().unwrap();
        let b2 = s2.next().await.unwrap().unwrap();
        let expected1: &[u8] = b"hello";
        let expected2: &[u8] = b" world";
        assert_eq!((&b1[..], &b2[..]), (expected1, expected2));
        assert!(s2.next().await.is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn combined_reader_yields_peek_then_tail() {
        let sb = SharedBody::new(8);
        let sb2 = sb.clone();

        let peek_buf = PeekBuf::from_slice(b"PEEK-");

        tokio::spawn(async move {
            sb2.push(Bytes::from_static(b"TAIL1"));
            sb2.push(Bytes::from_static(b"TAIL2"));
            sb2.finish();
        });

        let mut reader = SharedBody::combined_reader(peek_buf, Arc::new(sb));

        let mut out = Vec::new();
        reader.read_to_end(&mut out).await.unwrap();
        assert_eq!(&out[..], b"PEEK-TAIL1TAIL2");
    }
}
