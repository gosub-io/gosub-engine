use crate::emitter::NetObserver;
use crate::events::NetEvent;
use crate::net_types::{FetchResultMeta, NetError};
use crate::types::PeekBuf;
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

// This is the top of the response (HTTP headers + first 5KB of the body, if any), plus a stream
pub struct ResponseTop {
    /// Metadata about the result
    pub meta: FetchResultMeta,
    /// Peek buffer of the first PEEK_MAX of data
    pub peek_buf: PeekBuf,
    /// Stream reader to read the REMAINDER of the body (does NOT include peek buffer data)
    pub reader: Box<dyn AsyncRead + Unpin + Send>,
}

/// Makes a request to a given URL and returns the top of the response.
pub async fn fetch_response_top(
    client: Arc<reqwest::Client>,
    url: Url,
    cancel: CancellationToken,
    observer: Arc<dyn NetObserver + Send + Sync>,
) -> Result<ResponseTop, NetError> {
    let started = Instant::now();
    observer.on_event(NetEvent::Started { url: url.clone() });

    let resp = get_with_redirects(
        client.clone(),
        url.clone(),
        cancel.clone(),
        observer.clone(),
    )
    .await?;

    let mut meta = FetchResultMeta {
        final_url: resp.url().clone(),
        status: resp.status().as_u16(),
        status_text: resp.status().canonical_reason().unwrap_or("").to_string(),
        headers: resp.headers().clone(),
        content_length: resp.content_length(),
        content_type: resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string()),
        has_body: true,
    };

    let mut body_stream = resp
        .bytes_stream()
        .map_err(|e| NetError::Read(Arc::new(anyhow!(e))));
    let mut received_net: u64 = 0;
    let mut peek_buf_vec: Vec<u8> = Vec::with_capacity(PEEK_MAX);
    let mut excess: Option<Bytes> = None;

    let observer_clone = observer.clone();

    while peek_buf_vec.len() < PEEK_MAX {
        let next = tokio::select! {
            _ = cancel.cancelled() => {
                observer_clone.on_event(NetEvent::Cancelled { url: url.clone(), reason: "peek stream cancelled" });
                return Err(NetError::Cancelled("peek stream cancelled".into()));
            }
            n = body_stream.next() => n,
        };

        match next {
            Some(Ok(chunk)) => {
                received_net += chunk.len() as u64;

                observer.on_event(NetEvent::Progress {
                    received_bytes: received_net,
                    elapsed: started.elapsed(),
                    expected_length: meta.content_length,
                });

                let need = PEEK_MAX.saturating_sub(peek_buf_vec.len());
                if chunk.len() <= need {
                    peek_buf_vec.extend_from_slice(&chunk);
                } else {
                    peek_buf_vec.extend_from_slice(&chunk[..need]);
                    excess = Some(chunk.slice(need..));
                    break;
                }
            }
            Some(Err(e)) => {
                observer.on_event(NetEvent::Failed {
                    url: url.clone(),
                    error: e.into(),
                });
                return Err(NetError::Read(Arc::new(anyhow!("peek read failed"))));
            }
            None => {
                break;
            }
        }
    }

    let excess_len = excess.as_ref().map(|b| b.len() as u64).unwrap_or(0);

    let body_stream = if let Some(ex) = excess {
        stream::once(async move { Ok::<Bytes, NetError>(ex) })
            .chain(body_stream)
            .boxed()
    } else {
        body_stream.boxed()
    };

    let peek_buf = PeekBuf::from_vec(peek_buf_vec);
    let has_body_by_len = meta.content_length.unwrap_or(0) > 0 || !peek_buf.is_empty();
    meta.has_body = has_body_by_len;

    let stream = body_stream.map_err(|e: NetError| e.to_io());
    let inner_reader = StreamReader::new(stream);

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

/// Progress reader wraps another AsyncRead stream and emits progress events.
struct ProgressReader<R> {
    inner: R,
    cancel: CancellationToken,
    observer: Arc<dyn NetObserver + Send + Sync>,
    url: Url,
    started: Instant,
    expected_length: Option<u64>,
    received: u64,
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
        if self.cancel.is_cancelled() {
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

        let pre_len = buf.filled().len();
        let poll = Pin::new(&mut self.inner).poll_read(cx, buf);

        if let std::task::Poll::Ready(Ok(())) = &poll {
            let new_len = buf.filled().len();
            let read_bytes = (new_len - pre_len) as u64;

            if read_bytes == 0 {
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
    max_bytes: Option<usize>,
    read_idle_timeout: Duration,
    total_body_timeout: Option<Duration>,
) -> Result<(FetchResultMeta, Vec<u8>), NetError> {
    let started = Instant::now();

    let ResponseTop {
        meta,
        peek_buf,
        mut reader,
    } = fetch_response_top(client, url, cancel.clone(), observer.clone()).await?;

    let mut body_buf = peek_buf.to_vec();
    let mut chunk = [0u8; 16 * 1024];

    loop {
        if let Some(total) = total_body_timeout {
            if started.elapsed() > total {
                return Err(NetError::Timeout("total body timeout".into()));
            }
        }

        let n = tokio::select! {
            _ = cancel.cancelled() => {
                return Err(NetError::Cancelled("fetch_request_complete cancelled".into()));
            }
            r = timeout(read_idle_timeout, reader.read(&mut chunk)) => {
                match r {
                    Err(_) => return Err(NetError::Timeout("fetch_request_complete timeout".into())),
                    Ok(Err(e)) => return Err(NetError::Io(Arc::new(e))),
                    Ok(Ok(n)) => n,
                }
            }
        };

        if n == 0 {
            break;
        }

        if let Some(max) = max_bytes {
            if body_buf.len() + n > max {
                return Err(NetError::Read(Arc::new(anyhow!(
                    "fetch_request_complete exceeded maximum size of {} bytes",
                    max
                ))));
            }
        }

        body_buf.extend_from_slice(&chunk[..n]);
    }

    Ok((meta, body_buf))
}

/// Perform a GET request, following redirects up to MAX_REDIRECTS times.
async fn get_with_redirects(
    client: Arc<reqwest::Client>,
    url: Url,
    cancel: CancellationToken,
    observer: Arc<dyn NetObserver + Send + Sync>,
) -> Result<reqwest::Response, NetError> {
    let mut url = url;

    for _ in 0..MAX_REDIRECTS {
        let fut = client.get(url.clone()).send();
        tokio::pin!(fut);

        let resp = tokio::select! {
            _ = cancel.cancelled() => {
                observer.on_event(NetEvent::Cancelled { url: url.clone(), reason: "cancelled net.get_with_redirects" });
                return Err(NetError::Cancelled("cancelled net.get_with_redirects".into()));
            }
            r = &mut fut => r.context("net.get_with_redirects request failed").map_err(|e| NetError::Read(Arc::new(e)))?
        };

        if !resp.status().is_redirection() {
            return Ok(resp);
        }

        let status = resp.status().as_u16();
        let from = resp.url().clone();
        let loc = resp
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                NetError::Redirect(Arc::new(anyhow!(
                    "redirect status {} without Location header",
                    status
                )))
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
                let hdr = format!(
                    "HTTP/1.1 302 Found\r\nLocation: http://{}/big\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    addr
                );
                stream.write_all(hdr.as_bytes()).await?;
            }
            "/slow" => {
                let total = peek_max + 1024;
                let fast = peek_max;
                let slow = total - fast;

                let hdr = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
                    total
                );
                stream.write_all(hdr.as_bytes()).await?;
                stream.write_all(&vec![b'A'; fast]).await?;
                stream.flush().await?;
                tokio::time::sleep(Duration::from_millis(250)).await;
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
            loop {
                match listener.accept().await {
                    Ok((stream, _peer)) => {
                        let server_addr = addr;
                        tokio::spawn(async move {
                            let _ = handle_conn(stream, server_addr, super::PEEK_MAX).await;
                        });
                    }
                    Err(_) => break,
                }
            }
        });

        (base, handle)
    }

    fn test_client() -> reqwest::Client {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap()
    }

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
        } = super::fetch_response_top(client, url, cancel, observer)
            .await
            .unwrap();

        assert_eq!(
            peek_buf.len(),
            super::PEEK_MAX,
            "peek must be exactly PEEK_MAX"
        );
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

        let (meta, body) = super::fetch_response_complete(
            client,
            url,
            cancel,
            observer,
            None,
            Duration::from_secs(3),
            Some(Duration::from_secs(5)),
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

        let res = super::fetch_response_complete(
            client,
            url,
            cancel,
            observer,
            None,
            Duration::from_millis(100),
            Some(Duration::from_secs(2)),
        )
        .await;

        assert!(res.is_err(), "expected timeout error");
        let err = res.err().unwrap();
        let s = err.to_string().to_lowercase();
        assert!(
            s.contains("timeout"),
            "error should mention timeout, got: {s}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancel_during_peek_is_honored() {
        let (base, _jh) = spawn_test_server().await;

        let client = Arc::new(test_client());
        let url = base.join("slow").unwrap();
        let cancel = CancellationToken::new();
        let observer: Arc<dyn NetObserver + Send + Sync> = Arc::new(TestObserver);

        let cancel_clone = cancel.clone();
        let fut = super::fetch_response_top(client, url, cancel.clone(), observer);

        cancel_clone.cancel();

        let res = fut.await;
        assert!(res.is_err(), "expected cancellation");
        let s = res.err().unwrap().to_string().to_lowercase();
        assert!(s.contains("cancel"), "error should be cancellation: {s}");
    }
}
