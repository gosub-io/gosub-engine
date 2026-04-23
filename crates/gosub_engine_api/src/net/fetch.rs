use crate::engine::types::PeekBuf;
use crate::net::emitter::NetObserver;
use crate::net::events::NetEvent;
use crate::net::types::{FetchResultMeta, NetError};
use anyhow::{anyhow, Context};
use bytes::Bytes;
use futures_util::{stream, StreamExt, TryStreamExt};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::time::timeout;
use tokio_util::io::StreamReader;
use tokio_util::sync::CancellationToken;
use url::Url;

/// Peek buffer size (first bytes of body). Used for detecting mime type
const PEEK_MAX: usize = 5 * 1024;
/// Maximum number of redirects allowed
const MAX_REDIRECTS: usize = 20;

// This is the top of the response (HTTP headers + first 5KB of the body, if any), plus a stream (that starts from the peeked bytes)
pub struct ResponseTop {
    /// Metadata about the result
    pub meta: FetchResultMeta,
    /// Peek buffer of the first PEEK_MAX of data
    pub peek_buf: PeekBuf,
    /// Stream reader to read the REMAINDER of the body (this does NOT include peek buffer read data)
    pub reader: Box<dyn AsyncRead + Unpin + Send>,
}

/// This function will make a request to a given URL and returns the top of the response. These
/// are most likely the headers and the first 5 KB of body. This can be used to determine mime type
/// of the resource fetched. It will also return a stream reader that is able to read the remainder
/// of the body (minus the peek buffer).
pub async fn fetch_response_top(
    // Configured reqewst client to fetch the result
    client: Arc<reqwest::Client>,
    // Url to fetch (can be invalid or redirected)
    url: Url,
    // Cancel token to detect user cancellations
    cancel: CancellationToken,
    // Observer which can send out NetEvents to the UA
    observer: Arc<dyn NetObserver + Send + Sync>,
) -> Result<ResponseTop, NetError> {
    // Emit we are starting
    let started = Instant::now();
    observer.on_event(NetEvent::Started { url: url.clone() });

    // Get the response (GET requests only for now), and redirect if needed (300 requests)
    let resp = get_with_redirects(client.clone(), url.clone(), cancel.clone(), observer.clone()).await?;

    // Response is received, setup our meta structure
    let mut meta = FetchResultMeta {
        final_url: resp.url().clone(),
        status: resp.status().as_u16(),
        status_text: resp.status().canonical_reason().unwrap_or("").to_string(),
        headers: resp.headers().clone(),
        content_length: resp.content_length(), // More often than not, this is None
        content_type: resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string()),
        has_body: true, // Don't know yet
    };

    // Peek the stream up to PEEK_MAX bytes
    let mut body_stream = resp.bytes_stream().map_err(|e| NetError::Read(Arc::new(anyhow!(e))));
    let mut received_net: u64 = 0;
    let mut peek_buf_vec: Vec<u8> = Vec::with_capacity(PEEK_MAX);
    let mut excess: Option<Bytes> = None;

    let observer_clone = observer.clone();

    // We might need more fetches than one. Although it's unlikely unless you set PEEK_MAX to >8KB
    while peek_buf_vec.len() < PEEK_MAX {
        let next = tokio::select! {
            // Stream cancelled
            _ = cancel.cancelled() => {
                observer_clone.on_event(NetEvent::Cancelled { url: url.clone(), reason: "peek stream cancelled" });
                return Err(NetError::Cancelled("peek stream cancelled".into()));
            }
            // Read bytes from stream
            n = body_stream.next() => n,
        };

        match next {
            // We received a chunk of data
            Some(Ok(chunk)) => {
                received_net += chunk.len() as u64;

                observer.on_event(NetEvent::Progress {
                    received_bytes: received_net,
                    elapsed: started.elapsed(),
                    expected_length: meta.content_length,
                });

                let need = PEEK_MAX.saturating_sub(peek_buf_vec.len());
                if chunk.len() <= need {
                    // Entire chunk fits in our peek_buf.
                    peek_buf_vec.extend_from_slice(&chunk);
                } else {
                    // Chunk does not fit. For instance: Peek Buf = 12Kb. We read 8Kb in the first
                    // read, and 8kb in the second. In this case we have read 16kb when we only need
                    // the first 12kb. We fill the peek buf until full, and keep the rest in the
                    // 'excess' buffer
                    peek_buf_vec.extend_from_slice(&chunk[..need]);
                    excess = Some(chunk.slice(need..));
                    break;
                }
            }
            Some(Err(e)) => {
                // Something failed
                observer.on_event(NetEvent::Failed {
                    url: url.clone(),
                    error: e.into(),
                });
                return Err(NetError::Read(Arc::new(anyhow!("peek read failed"))));
            }
            None => {
                // Stream ended successfully
                break;
            }
        }
    }

    // Save the length before we store the excess into a body stream
    let excess_len = excess.as_ref().map(|b| b.len() as u64).unwrap_or(0);

    // It's possible that we have read too much, and we have an exccess buffer, so we create
    // a new stream that starts at the end of the peek buffer WITH the excess buffer in front.
    //
    //  |--- Peek buffer ---|---- Excess buffer ----| ---- body stream ----|
    //                                              ^ stream starts here
    //                      ^  new body stream "rereads" the excess buffer and starts here
    let body_stream = if let Some(ex) = excess {
        stream::once(async move { Ok::<Bytes, NetError>(ex) })
            .chain(body_stream)
            .boxed()
    } else {
        body_stream.boxed()
    };

    // Update last remaining items in meta struct
    let peek_buf = PeekBuf::from_vec(peek_buf_vec);
    let has_body_by_len = meta.content_length.unwrap_or(0) > 0 || !peek_buf.is_empty();
    meta.has_body = has_body_by_len;

    // Wrap our body stream into a progress reader. This way it will emit net events to the observer
    // whenever it is read.
    let stream = body_stream.map_err(|e: NetError| e.to_io());
    let inner_reader = StreamReader::new(stream);

    // Update the progress counter to the point of the bytes read (note: this can cause a strange
    // decrease in bytes read in the progress events?)
    let already_delivered = received_net - excess_len;

    let progress_reader = ProgressReader::new(
        inner_reader,
        cancel.clone(),
        observer.clone(),
        url.clone(),
        started,
        meta.content_length,
        already_delivered,
    );

    Ok(ResponseTop {
        meta,
        peek_buf,
        reader: Box::new(progress_reader),
    })
}

/// Progres reader is a simple stream that will wrap another AsyncRead stream, and emit progress
/// events to the observer.
struct ProgressReader<R> {
    /// Actual reader
    inner: R,
    /// Cancellation token
    cancel: CancellationToken,
    // Observer to emit events to
    observer: Arc<dyn NetObserver + Send + Sync>,
    /// Url we are reading from. For event emission
    url: Url,
    /// When we started reading, since we already read the peek buffer from this stream
    started: Instant,
    /// Expected length of the resource, if known
    expected_length: Option<u64>,
    /// Number of bytes already received (from the peek buffer)
    received: u64,
    /// Whether we already emitted a cancelled event
    cancel_emitted: bool,
}

impl<R: AsyncRead + Unpin> ProgressReader<R> {
    fn new(
        inner: R,
        cancel: CancellationToken,
        observer: Arc<dyn NetObserver + Send + Sync>,
        url: Url,
        started: Instant,
        expected_length: Option<u64>,
        already_received: u64,
    ) -> Self {
        Self {
            inner,
            cancel,
            observer,
            url,
            started,
            expected_length,
            received: already_received,
            cancel_emitted: false,
        }
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for ProgressReader<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        // When cancelled, we are directly done
        if self.cancel.is_cancelled() {
            // Maybe it's already cancelled? Then don't send another cancelled event
            if !self.cancel_emitted {
                self.observer.on_event(NetEvent::Cancelled {
                    url: self.url.clone(),
                    reason: "progress reader cancelled",
                });
                self.cancel_emitted = true;
            }

            let err = NetError::Cancelled("progress reader cancelled".into());
            return std::task::Poll::Ready(Err(err.to_io()));
        }

        // Pull new bytes from the reader
        let pre_len = buf.filled().len();
        let poll = Pin::new(&mut self.inner).poll_read(cx, buf);

        if let std::task::Poll::Ready(Ok(())) = &poll {
            let new_len = buf.filled().len();
            let read_bytes = (new_len - pre_len) as u64;

            // nothing read, then we have reached the end of the stream
            if read_bytes == 0 {
                // Finished
                self.observer.on_event(NetEvent::Finished {
                    received_bytes: self.received,
                    elapsed: self.started.elapsed(),
                    url: self.url.clone(),
                });
            }
            if read_bytes > 0 {
                self.received += read_bytes;
                self.observer.on_event(NetEvent::Progress {
                    received_bytes: self.received,
                    elapsed: self.started.elapsed(),
                    expected_length: self.expected_length,
                });
            }
        }

        poll
    }
}

/// Fetch a complete resource, returning the metadata and the full body as a Vec<u8>.
pub async fn fetch_response_complete(
    client: Arc<reqwest::Client>,
    url: Url,
    cancel: CancellationToken,
    observer: Arc<dyn NetObserver + Send + Sync>,
    // We can cap the amount of bytes we want to read (None for unlimited)
    max_bytes: Option<usize>,
    // Maximum time allowed between reads
    read_idle_timeout: Duration,
    // Total time of read allowed (if any)
    total_body_timeout: Option<Duration>,
) -> Result<(FetchResultMeta, Vec<u8>), NetError> {
    let started = Instant::now();

    let ResponseTop {
        meta,
        peek_buf,
        mut reader,
    } = fetch_response_top(client, url, cancel.clone(), observer.clone()).await?;

    // We don't care about the peek buffer. We just create a new buffer and read the rest of the
    // stream into it.
    let mut body_buf = peek_buf.to_vec();
    let mut chunk = [0u8; 16 * 1024];

    loop {
        // Check if we hit the total body timeout
        if let Some(total) = total_body_timeout {
            if started.elapsed() > total {
                return Err(NetError::Timeout("total body timeout".into()));
            }
        }

        let n = tokio::select! {
            // Stream cancelled
            _ = cancel.cancelled() => {
                return Err(NetError::Cancelled("fetch_request_complete cancelled".into()));
            }
            // Read bytes, or timeout when not read something in time
            r = timeout(read_idle_timeout, reader.read(&mut chunk)) => {
                match r {
                    Err(_) => return Err(NetError::Timeout("fetch_request_complete timeout".into())),
                    Ok(Err(e)) => return Err(NetError::Io(Arc::new(e))),
                    Ok(Ok(n)) => n,
                }
            }
        };

        if n == 0 {
            // Stream ended normally
            break;
        }

        if let Some(max) = max_bytes {
            // Too many bytes are read. We throw an error (@TODO: should we do this? not just cap
            // the buffer and return that?
            if body_buf.len() + n > max {
                return Err(NetError::Read(Arc::new(anyhow!(
                    "fetch_request_complete exceeded maximum size of {} bytes",
                    max
                ))));
            }
        }

        // Exctent the bufferr with read chunk
        body_buf.extend_from_slice(&chunk[..n]);
    }

    Ok((meta, body_buf))
}

/// Perform a GET request, following redirects up to MAX_REDIRECTS times, while sending out net events
async fn get_with_redirects(
    client: Arc<reqwest::Client>,
    url: Url,
    cancel: CancellationToken,
    observer: Arc<dyn NetObserver + Send + Sync>,
) -> Result<reqwest::Response, NetError> {
    let mut url = url;

    // We cap the number of redirects in order to prevent redirection loops
    for _ in 0..MAX_REDIRECTS {
        // Get response from the client. Note that we need to pin the future as we are using it in select!
        let fut = client.get(url.clone()).send();
        tokio::pin!(fut);

        let resp = tokio::select! {
            // cancelled
            _ = cancel.cancelled() => {
                observer.on_event(NetEvent::Cancelled { url: url.clone(), reason: "cancelled net.get_with_redirects" });
                return Err(NetError::Cancelled("cancelled net.get_with_redirects".into()));
            }
            // Read response or fail
            r = &mut fut => r.context("net.get_with_redirects request failed").map_err(|e| NetError::Read(Arc::new(e)))?
        };

        // Not a redirection, return response directly
        if !resp.status().is_redirection() {
            return Ok(resp);
        }

        // 3xx response detected, follow the redirect found in the Location header
        let status = resp.status().as_u16();
        let from = resp.url().clone();
        let loc = resp
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                NetError::Redirect(Arc::new(anyhow!("redirect status {} without Location header", status)))
            })?;

        let to = from
            .join(loc)
            .map_err(|e| NetError::Redirect(Arc::new(anyhow!("invalid redirect URL '{}': {}", loc, e))))?;

        observer.on_event(NetEvent::Redirected {
            from,
            to: to.clone(),
            status,
        });

        url = to
    }

    Err(NetError::Redirect(Arc::new(anyhow!("too many redirects"))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cow_utils::CowUtils;
    use std::io;
    use std::net::SocketAddr;
    use std::time::Duration;
    use tokio::io::AsyncWriteExt as _;
    use tokio::net::{TcpListener, TcpStream};
    use tokio_util::sync::CancellationToken;

    struct TestObserver;

    impl NetObserver for TestObserver {
        fn on_event(&self, _event: NetEvent) {}
    }

    async fn read_request(stream: &mut TcpStream) -> io::Result<String> {
        let mut buf = Vec::with_capacity(1024);
        let mut tmp = [0u8; 512];
        loop {
            let n = stream.read(&mut tmp).await?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
            if buf.len() > 16 * 1024 {
                break;
            }
        }
        Ok(String::from_utf8_lossy(&buf).to_string())
    }

    fn parse_path(req: &str) -> String {
        if let Some(line) = req.lines().next() {
            if let Some(rest) = line.strip_prefix("GET ") {
                if let Some(idx) = rest.find(' ') {
                    return rest[..idx].to_string();
                }
            }
        }
        "/".to_string()
    }

    async fn handle_conn(mut stream: TcpStream, addr: SocketAddr, peek_max: usize) -> io::Result<()> {
        let req = read_request(&mut stream).await?;
        let path = parse_path(&req);

        match path.as_str() {
            "/big" => {
                // 12 KiB body to test excess logic
                let len = 12 * 1024;
                let body = vec![b'X'; len];
                let hdr = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    len
                );
                stream.write_all(hdr.as_bytes()).await?;
                stream.write_all(&body).await?;
            }
            "/redirect" => {
                // Redirect to /big on same host
                let hdr = format!(
                    "HTTP/1.1 302 Found\r\nLocation: http://{}/big\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    addr
                );
                stream.write_all(hdr.as_bytes()).await?;
            }
            "/slow" => {
                // Send exactly PEEK_MAX bytes fast, then stall (to trigger read idle timeout later)
                let total = peek_max + 1024;
                let fast = peek_max;
                let slow = total - fast;

                let hdr = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
                    total
                );
                stream.write_all(hdr.as_bytes()).await?;
                // fast prefix
                stream.write_all(&vec![b'A'; fast]).await?;
                stream.flush().await?;
                // stall long enough to exceed typical idle timeout in tests
                tokio::time::sleep(Duration::from_millis(250)).await;
                // write a tiny bit so connection stays alive (not strictly needed)
                let _ = stream.write_all(&vec![b'B'; slow]).await;
            }
            _ => {
                let body = b"hello";
                let hdr = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(hdr.as_bytes()).await?;
                stream.write_all(body).await?;
            }
        }
        Ok(())
    }

    async fn spawn_test_server() -> (Url, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let base = Url::parse(&format!("http://{}/", addr)).unwrap();

        let handle = tokio::spawn(async move {
            while let Ok((stream, _peer)) = listener.accept().await {
                let server_addr = addr;
                tokio::spawn(async move {
                    let _ = handle_conn(stream, server_addr, super::PEEK_MAX).await;
                });
            }
        });

        (base, handle)
    }

    fn test_client() -> reqwest::Client {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none()) // we do redirects ourselves
            .build()
            .unwrap()
    }

    // ---------- Tests ----------

    #[tokio::test(flavor = "current_thread")]
    async fn top_returns_peek_and_reader_rest() {
        let (base, _jh) = spawn_test_server().await;

        let client = Arc::new(test_client());
        let url = base.join("big").unwrap();
        let cancel = CancellationToken::new();
        let observer: Arc<dyn NetObserver + Send + Sync> = Arc::new(TestObserver);

        let ResponseTop {
            meta,
            peek_buf,
            mut reader,
        } = super::fetch_response_top(client, url, cancel, observer).await.unwrap();

        assert_eq!(peek_buf.len(), super::PEEK_MAX, "peek must be exactly PEEK_MAX");
        // Read remainder
        let mut rest = Vec::new();
        reader.read_to_end(&mut rest).await.unwrap();

        assert_eq!(peek_buf.len() + rest.len(), 12 * 1024);
        assert!(meta.has_body);
        assert_eq!(meta.status, 200);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn redirects_are_followed() {
        let (base, _jh) = spawn_test_server().await;

        let client = Arc::new(test_client());
        let url = base.join("redirect").unwrap();
        let cancel = CancellationToken::new();
        let observer: Arc<dyn NetObserver + Send + Sync> = Arc::new(TestObserver);

        // Small generous timeouts so the test is stable
        let (meta, body) = super::fetch_response_complete(
            client,
            url,
            cancel,
            observer,
            None,                         // max_bytes
            Duration::from_secs(3),       // read_idle_timeout
            Some(Duration::from_secs(5)), // total_body_timeout
        )
        .await
        .unwrap();

        assert_eq!(meta.status, 200);
        assert_eq!(body.len(), 12 * 1024);
        assert!(meta.has_body);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn idle_timeout_triggers_on_slow_body() {
        let (base, _jh) = spawn_test_server().await;

        let client = Arc::new(test_client());
        let url = base.join("slow").unwrap();
        let cancel = CancellationToken::new();
        let observer: Arc<dyn NetObserver + Send + Sync> = Arc::new(TestObserver);

        // Set idle timeout < server stall (250ms)
        let res = super::fetch_response_complete(
            client,
            url,
            cancel,
            observer,
            None,
            Duration::from_millis(100),   // read_idle_timeout
            Some(Duration::from_secs(2)), // total_body_timeout
        )
        .await;

        assert!(res.is_err(), "expected timeout error");
        let err = res.err().unwrap();
        let s = err.to_string().cow_to_ascii_lowercase().into_owned();
        assert!(s.contains("timeout"), "error should mention timeout, got: {s}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancel_during_peek_is_honored() {
        let (base, _jh) = spawn_test_server().await;

        let client = Arc::new(test_client());
        let url = base.join("slow").unwrap();
        let cancel = CancellationToken::new();
        let observer: Arc<dyn NetObserver + Send + Sync> = Arc::new(TestObserver);

        // Kick off, then cancel quickly; server sends headers + PEEK_MAX fast, but we cancel immediately
        let cancel_clone = cancel.clone();
        let fut = super::fetch_response_top(client, url, cancel.clone(), observer);

        // Cancel immediately
        cancel_clone.cancel();

        let res = fut.await;
        assert!(res.is_err(), "expected cancellation");
        let s = res.err().unwrap().to_string().cow_to_ascii_lowercase().into_owned();
        assert!(s.contains("cancel"), "error should be cancellation: {s}");
    }
}
