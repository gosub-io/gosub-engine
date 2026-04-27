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

use crate::engine::types::PeekBuf;
use crate::net::types::NetError;
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
///
/// # Examples
///
/// Basic broadcast to two subscribers:
/// ```ignore
/// # use bytes::Bytes;
/// # use futures_util::StreamExt;
/// # use std::sync::Arc;
/// # use gosub_engine::net::shared_body::SharedBody;
/// let sb = SharedBody::new(8);
/// let mut a = sb.subscribe_stream();
/// let mut b = sb.subscribe_stream();
///
/// sb.push(Bytes::from_static(b"hi"));
/// sb.finish();
///
/// # tokio_test::block_on(async {
/// assert_eq!(&a.next().await.unwrap().unwrap()[..], b"hi");
/// assert!(a.next().await.is_none());
/// assert_eq!(&b.next().await.unwrap().unwrap()[..], b"hi");
/// assert!(b.next().await.is_none());
/// # });
/// ```
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
    ///
    /// Each subscriber gets a queue with this capacity. When full, the slow
    /// subscriber is dropped rather than applying backpressure to the producer.
    ///
    /// A capacity of **1–4** keeps latency low; **32+** favors throughput.
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
    ///
    /// - If a subscriber's queue is **full** or **closed**, that subscriber is removed.
    /// - If [`finish`](Self::finish) or [`error`](Self::error) has been called,
    ///   additional pushes are ignored.
    pub fn push(&self, chunk: Bytes) {
        let (subs, mut to_remove) = {
            let st = self.inner.lock().unwrap();
            if st.closed {
                return;
            }
            let subs: Vec<(u64, mpsc::Sender<_>)> = st.subs.iter().map(|(id, tx)| (*id, tx.clone())).collect();

            // It might be possible that we can detect closed channels here and thus add them
            // already to the remove list. For now, we just return an empty list as a starting point.
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
    ///
    /// After this call:
    /// - The next item each subscriber receives is `Err(e.clone())`.
    /// - The stream then ends (`None`).
    /// - New subscribers will see an **empty** stream.
    pub fn error(&self, e: NetError) {
        // drain and drop under lock; send error outside the lock
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
    ///
    /// Dropping all senders causes subscribers to yield `None`. New subscribers
    /// will see an empty stream.
    pub fn finish(&self) {
        // closed -> drop all senders so receivers see EOF
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

    /// Subscribes **from now on**, returning a stream of body chunks.
    ///
    /// Chunks produced **before** subscribing are **not** replayed.
    ///
    /// See also [`subscribe_stream`](Self::subscribe_stream) for using the
    /// default capacity configured at `SharedBody` creation.
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
    ///
    /// The capacity is the `max_queue` value that was provided to [`new`](Self::new).
    pub fn subscribe_stream(&self) -> BoxStream<'static, Result<Bytes, NetError>> {
        let cap = {
            let st = self.inner.lock().unwrap();
            st.max_queue
        };

        self.subscribe_with_cap(cap)
    }

    /// Returns an `AsyncRead` that yields `peek` first, then the live tail bytes.
    ///
    /// This is useful when downstream code expects an `AsyncRead` instead of a
    /// `Stream` (e.g., `tokio::io::copy`). The returned reader:
    ///
    /// 1. Reads the provided `peek` buffer.
    /// 2. Continues with chunks from `shared.subscribe_stream()`.
    ///
    /// # Example
    /// ```ignore
    /// # use std::{pin::Pin, sync::Arc};
    /// # use tokio::io::{self, AsyncReadExt};
    /// # use gosub_engine::net::shared_body::SharedBody;
    /// let shared = Arc::new(SharedBody::new(8));
    /// let mut r = SharedBody::combined_reader(b"HEAD".to_vec(), shared.clone());
    /// # tokio_test::block_on(async {
    /// let mut out = Vec::new();
    /// r.read_to_end(&mut out).await.unwrap();
    /// # let _ = out;
    /// # });
    /// ```
    pub fn combined_reader(peek_buf: PeekBuf, shared: Arc<SharedBody>) -> Pin<Box<dyn AsyncRead + Send>> {
        let head = stream::iter([Ok::<Bytes, std::io::Error>(peek_buf.into_bytes())]);
        let rest_stream = shared.subscribe_stream().map_err(|e: NetError| e.to_io());

        let combined = head.chain(rest_stream);
        Box::pin(StreamReader::new(combined))
    }
}

/// Options to wrap an `AsyncRead` into a `SharedBody` via
/// [`SharedBody::from_reader`].
///
/// These control buffering, cancellation, timeouts, and byte limits.
pub struct ReaderOptions {
    /// Per-subscriber queue capacity for the `SharedBody` created by
    /// [`from_reader`](SharedBody::from_reader).
    ///
    /// Larger values increase tolerance for short subscriber stalls, at the cost
    /// of memory. Small values drop lagging subscribers sooner.
    pub capacity: usize,
    /// Size of the temporary read buffer used when pulling from the source
    /// `AsyncRead`. Larger buffers reduce syscalls but may raise latency per chunk.
    pub buf_size: usize,
    /// Optional cooperative cancellation token. If cancelled, reading stops and
    /// subscribers receive `NetError::Cancelled`.
    pub cancel: Option<CancellationToken>,
    /// Maximum allowed time between successful read operations. When exceeded,
    /// reading stops with `NetError::Timeout("read idle timeout")`.
    pub idle_timeout: Option<Duration>,
    /// Total deadline for the entire body. When exceeded, reading stops with
    /// `NetError::Timeout("total read timeout")`.
    pub total_timeout: Option<Duration>,
    /// Maximum total number of bytes to read. Exceeding this limit triggers an
    /// error and closes the stream.
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
    /// a new `SharedBody`, honoring cancellation, timeouts, and size limits.
    ///
    /// - On EOF: calls [`finish`](Self::finish).
    /// - On I/O error or policy violation: calls [`error`](Self::error).
    ///
    /// The returned `Arc<SharedBody>` can be subscribed to immediately; chunks
    /// will arrive as the background task reads.
    ///
    /// # Examples
    /// Wrap a `reqwest` body (converted to `AsyncRead`) and tee it:
    /// ```ignore
    /// # use std::sync::Arc;
    /// # use futures_util::TryStreamExt;
    /// # use gosub_engine::net::shared_body::{SharedBody, ReaderOptions};
    /// # async fn demo(mut r: impl tokio::io::AsyncRead + Send + Unpin + 'static) {
    /// let body = SharedBody::from_reader(r, ReaderOptions::default());
    /// let mut a = body.subscribe_stream();
    /// let mut b = body.subscribe_stream();
    ///
    /// // A: count bytes
    /// tokio::spawn(async move {
    ///     let mut total = 0usize;
    ///     while let Some(chunk) = a.next().await {
    ///         total += chunk.unwrap().len();
    ///     }
    ///     println!("A total={}", total);
    /// });
    ///
    /// // B: collect whole body
    /// tokio::spawn(async move {
    ///     let collected = b.try_fold(Vec::new(), |mut acc, bytes| async move {
    ///         acc.extend_from_slice(&bytes);
    ///         Ok(acc)
    ///     }).await.unwrap();
    ///     println!("B len={}", collected.len());
    /// });
    /// # }
    /// ```
    pub fn from_reader<R>(mut reader: R, opts: ReaderOptions) -> Arc<Self>
    where
        R: AsyncRead + Send + 'static + Unpin,
    {
        let sb = Arc::new(SharedBody::new(opts.capacity));
        let sb_clone = sb.clone();

        // read in background
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
            let mut total_read: u64 = 0; // Does NOT take into account the peek buf!

            // Some helper functions
            let check_total_deadline = |now: Instant| -> Result<(), NetError> {
                if let Some(dl) = deadline {
                    if now >= dl {
                        return Err(NetError::Timeout("total read timeout".to_string()));
                    }
                }
                Ok(())
            };

            // Make sure we haven't already exceeded the total deadline
            if let Err(e) = check_total_deadline(Instant::now()) {
                sb_clone.error(e);
                return;
            }

            loop {
                // User cancelled the read
                if cancel.is_cancelled() {
                    sb_clone.error(NetError::Cancelled("read cancelled".to_string()));
                    return;
                }

                // Check how much we can read this iteration
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

                // Read with optional idle timeout
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
                        // Eof
                        sb_clone.finish();
                        return;
                    }
                    Ok(n) => {
                        // Read n bytes
                        // last_progress = Instant::now();
                        total_read = total_read.saturating_add(n as u64);

                        // Push to shared body
                        sb_clone.push(Bytes::copy_from_slice(&buf[..n]));

                        // Did we hit the max size limit?
                        if let Some(max) = max_size {
                            if total_read > max {
                                sb_clone.error(NetError::Io(Arc::new(std::io::Error::other(
                                    "max size exceeded during read",
                                ))));
                                return;
                            }
                        }

                        // Did we hit the total deadline?
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

/// Per-subscriber stream returned by [`SharedBody::subscribe_*`].
///
/// Deregisters itself from the parent `SharedBody` on drop. You normally do not
/// use `SubStream` directly—treat it as an opaque `Stream<Item = Result<Bytes, NetError>>`.
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

        // s1 sees both chunks then EOF (None)
        let a1 = s1.next().await.unwrap().unwrap();
        let a2 = s1.next().await.unwrap().unwrap();
        let expected1: &[u8] = b"hello";
        let expected2: &[u8] = b" world";
        assert_eq!((&a1[..], &a2[..]), (expected1, expected2));
        assert!(s1.next().await.is_none());

        // s2 sees both chunks then EOF
        let b1 = s2.next().await.unwrap().unwrap();
        let b2 = s2.next().await.unwrap().unwrap();
        let expected1: &[u8] = b"hello";
        let expected2: &[u8] = b" world";
        assert_eq!((&b1[..], &b2[..]), (expected1, expected2));
        assert!(s2.next().await.is_none());
    }

    // This test ensures that a slow subscriber is dropped when it can't
    // keep up with the producer.
    #[tokio::test(flavor = "current_thread")]
    async fn shared_body_drops_slow_subscriber() {
        // Small per-sub queue to force lag quickly. Only one item can be buffered.
        let sb = SharedBody::new(1);

        // Slow stream will read "slowly"
        let mut slow = sb.subscribe_stream();
        // Fast stream will read "quickly" (ie: we read before pushing more)
        let mut fast = sb.subscribe_stream();

        // Push first chunk; both get it buffered.
        sb.push(Bytes::from_static(b"A"));

        // Fast stream reads the 'A' quickly
        let fa = fast.next().await.unwrap().unwrap();

        // Fast is drained. Slow is full and thus dropped
        sb.push(Bytes::from_static(b"B"));

        // Consume 'B' from fast
        let fb = fast.next().await.unwrap().unwrap();

        // Slow should get 'A' but not 'B' (it was dropped)
        let first = slow.next().await.unwrap();
        assert!(first.is_ok());
        let tail = slow.next().await; // None or Err
        assert!(tail.is_none() || tail.unwrap().is_err());

        // Push third chunk; There is no more slow subscriber, only fast.
        sb.push(Bytes::from_static(b"C"));

        // fast should get the remaining 'C'
        let fc = fast.next().await.unwrap().unwrap();

        // Fast should have all three chunks
        let exp1: &[u8] = b"A";
        let exp2: &[u8] = b"B";
        let exp3: &[u8] = b"C";
        assert_eq!((&fa[..], &fb[..], &fc[..]), (exp1, exp2, exp3));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn combined_reader_yields_peek_then_tail() {
        let sb = SharedBody::new(8);
        let sb2 = sb.clone();

        let peek_buf = PeekBuf::from_slice(b"PEEK-");

        // write tail in background
        tokio::spawn(async move {
            sb2.push(Bytes::from_static(b"TAIL1"));
            sb2.push(Bytes::from_static(b"TAIL2"));
            sb2.finish();
        });

        // use the static helper you defined
        let mut reader = SharedBody::combined_reader(peek_buf, Arc::new(sb));

        let mut out = Vec::new();
        reader.read_to_end(&mut out).await.unwrap();
        assert_eq!(&out[..], b"PEEK-TAIL1TAIL2");
    }
}
