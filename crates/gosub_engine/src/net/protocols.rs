//! Protocols the engine speaks itself, beside the http(s) of gosub-sonar.
//!
//! A protocol here is a scheme the network stack answers with its own client: it opens the
//! connection, speaks the wire format and hands back a body and its media type. Unlike
//! `file:` and `data:`, which the I/O runtime answers before a request goes anywhere, these
//! take the ordinary fetch path, so the request is placed on the private-network policy like
//! any other and, with `security.network_process` on, is served from the network process.
//!
//! The set is fixed at build time: each protocol is a cargo feature and an entry in
//! [`PROTOCOLS`]. Nothing is registered at run time, because the network process runs none
//! of the embedder's code.
//!
//! Policy, enforced by the broker in [`refuse`] before the request goes anywhere: a protocol
//! is off when its setting is off, and serves top-level navigations only. A page cannot load
//! a subresource over one, so a remote document never gets to drive a raw-socket protocol.
//! A page may still link to one from a page on the same protocol (`page_may_navigate`), and
//! such a navigation reaches no further than that page's space
//! (`TabIdentity::protocol_navigation_reach`). What passes is served by [`run`], in this
//! process or in the network process.
//!
//! A protocol's fetch builds on [`socket`] for its connection, which holds every address to
//! the reach and every port to [`socket::port_refused`], and on [`page`] for the HTML it
//! shows. [`is_network_place`] and [`transport`] answer, for any URL, what an embedder asks
//! of a scheme (bookmarks, the address bar's padlock), so an embedder names no schemes.
//!
//! A protocol that trusts certificates on first use learns the pin the zone keeps for each
//! server it reaches ([`ProtocolRequest::pins`]) and reports the certificates first visits
//! showed ([`ProtocolResponse::certificates`]); the broker records them ([`known_hosts`]).

pub mod identities;
pub mod known_hosts;
pub mod page;
pub mod socket;

use crate::net::protocols::known_hosts::{Pins, ServerPin};
use crate::net::ssrf::AddressSpace;
use crate::net::types::{FetchRequest, FetchResult, FetchResultMeta, NetError};
use async_trait::async_trait;
use bytes::Bytes;
use gosub_sonar::net::events::NetEvent;
use gosub_sonar::net::observer::NetObserver;
use gosub_sonar::net::shared_body::{ReaderOptions, SharedBody};
use gosub_sonar::net::types::{BlockReason, ResourceKind as NetResourceKind};
use gosub_sonar::types::PeekBuf;
use http::HeaderMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use url::Url;

/// What a protocol is asked to fetch.
#[derive(Clone, Copy)]
pub struct ProtocolRequest<'a> {
    pub url: &'a Url,
    /// The most private address space the fetch may reach.
    pub reach: AddressSpace,
    /// Where the certificates the zone has seen are learned, for each server the fetch
    /// reaches. `None` for a protocol that pins nothing; every visit is a first one then.
    pub pins: Option<&'a dyn Pins>,
    /// The client certificate the request presents, where its scope covers the URL
    /// (`identities`): on a redirect only while it stays inside that scope.
    pub identity: Option<&'a crate::net::protocols::identities::ClientIdentity>,
}

impl std::fmt::Debug for ProtocolRequest<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProtocolRequest")
            .field("url", &self.url.as_str())
            .field("reach", &self.reach)
            .field("pins", &self.pins.is_some())
            .field("identity", &self.identity.map(|identity| &identity.scope))
            .finish()
    }
}

/// What a protocol answered with.
#[derive(Debug)]
pub struct ProtocolResponse {
    /// Media type of `body`, as the rest of the pipeline routes on it.
    pub content_type: String,
    /// The whole body, or its first bytes when `rest` follows.
    pub body: Bytes,
    /// The rest of the body, still arriving: served as a stream ([`stream_body`]).
    pub rest: Option<BodyStream>,
    /// The peer the body was read from: what the document is placed by on the
    /// private-network policy, as for an http response.
    pub peer_addr: Option<SocketAddr>,
    /// The certificates servers showed on a first visit, one for each, for the broker to pin.
    pub certificates: Vec<ServerPin>,
    /// Where `body` came from, when a redirect the protocol followed led elsewhere than the
    /// request's URL: what the page commits under and resolves its links against.
    pub final_url: Option<Url>,
}

/// The rest of a response body, read as it arrives.
pub struct BodyStream {
    pub reader: Box<dyn tokio::io::AsyncRead + Send + Unpin>,
    /// Longest the body may go without a byte before it ends in a timeout.
    pub idle: std::time::Duration,
}

impl std::fmt::Debug for BodyStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BodyStream")
            .field("idle", &self.idle)
            .finish_non_exhaustive()
    }
}

/// What a protocol failed with, and the certificates first visits showed before it did: a
/// server that answered is pinned even when the request goes on to fail, as one asking for a
/// client certificate does.
#[derive(Debug)]
pub struct ProtocolError {
    pub error: NetError,
    pub certificates: Vec<ServerPin>,
}

impl From<NetError> for ProtocolError {
    fn from(error: NetError) -> Self {
        Self {
            error,
            certificates: Vec::new(),
        }
    }
}

/// What [`run`] served: the reply for the requester, and the certificates first visits showed.
#[derive(Debug)]
pub struct Served {
    pub result: FetchResult,
    pub certificates: Vec<ServerPin>,
}

/// A scheme the engine fetches with its own client.
#[async_trait]
pub trait Protocol: Send + Sync {
    /// The URL scheme served, without the colon.
    fn scheme(&self) -> &'static str;

    /// The settings-store key that turns the protocol off. Read by the broker, since the
    /// network process has no settings of its own.
    fn setting(&self) -> &'static str;

    /// The port a URL without one connects to. Open to this protocol even when the Fetch
    /// standard blocks it ([`socket::port_refused`]).
    fn default_port(&self) -> u16;

    /// Whether the connection is encrypted, as an address bar shows it.
    fn encrypted(&self) -> bool;

    /// Whether the protocol trusts certificates on first use: learns the zone's pins for the
    /// servers it reaches and reports what first visits saw. Only such a protocol's report
    /// is pinned.
    fn pins_certificates(&self) -> bool {
        false
    }

    /// The query a form submitted with GET to this protocol's URLs carries, from the form's
    /// data (name and value, in order). `None` keeps the web's `name=value&...` encoding.
    fn form_query(&self, _data: &[(String, String)]) -> Option<String> {
        None
    }

    /// Fetch `request.url`, reaching no address space more private than `request.reach`.
    /// Resolves when the whole body is in, or its first bytes for a body that streams
    /// ([`ProtocolResponse::rest`]); dropping the future abandons the fetch.
    async fn fetch(&self, request: ProtocolRequest<'_>) -> Result<ProtocolResponse, ProtocolError>;
}

/// Every protocol compiled into this build.
static PROTOCOLS: &[&dyn Protocol] = &[
    #[cfg(feature = "gopher")]
    &crate::net::gopher::Gopher,
    #[cfg(feature = "finger")]
    &crate::net::finger::Finger,
    #[cfg(feature = "gemini")]
    &crate::net::gemini::Gemini,
];

/// The protocol serving `scheme`, if this build has one.
pub fn lookup(scheme: &str) -> Option<&'static dyn Protocol> {
    PROTOCOLS.iter().copied().find(|p| p.scheme() == scheme)
}

/// How a page at a URL came over the network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Encrypted,
    Cleartext,
}

/// How `url` is fetched: https and encrypted engine protocols are [`Transport::Encrypted`],
/// http and the rest of the engine protocols [`Transport::Cleartext`]. `None` for what does
/// not come over the network (`file:`, `data:`, `about:`, internal pages).
pub fn transport(url: &Url) -> Option<Transport> {
    let encrypted = match url.scheme() {
        "https" => true,
        "http" => false,
        scheme => lookup(scheme)?.encrypted(),
    };
    Some(if encrypted {
        Transport::Encrypted
    } else {
        Transport::Cleartext
    })
}

/// Whether `url` is a place on the network: http(s), or a scheme an engine protocol in this
/// build serves. What history records a visit to and an embedder may bookmark.
pub fn is_network_place(url: &Url) -> bool {
    transport(url).is_some()
}

/// The protocol serving this request, if this build has one.
pub fn for_request(req: &FetchRequest) -> Option<&'static dyn Protocol> {
    lookup(req.url.scheme())
}

/// Why a request for a protocol's scheme is refused, if it is.
pub fn refusal(req: &FetchRequest, enabled: bool) -> Option<BlockReason> {
    if !enabled || req.kind != NetResourceKind::Primary {
        return Some(BlockReason::UrlPolicy);
    }
    None
}

/// Refuse a request the policy does not allow, emitting the events of a blocked fetch, and
/// return the error for the reply channel. `None` when the request may go ahead.
pub fn refuse(
    protocol: &dyn Protocol,
    req: &FetchRequest,
    enabled: bool,
    observer: &dyn NetObserver,
) -> Option<FetchResult> {
    let reason = refusal(req, enabled)?;
    let url = req.url.clone();
    log::warn!("refusing {} request for {url} (kind {:?})", protocol.scheme(), req.kind);
    observer.on_event(NetEvent::Accepted {
        url: url.clone(),
        joined: false,
    });
    observer.on_event(NetEvent::Blocked {
        url: url.clone(),
        reason,
    });
    Some(FetchResult::Error(NetError::Blocked { reason, url }))
}

/// Serve a request [`refuse`] let through, emitting the observer events a network fetch
/// would, and return the response (or error) for the reply channel: buffered, or a stream
/// when the protocol left the rest of the body to arrive. `None` when the fetch was
/// cancelled: the reply channel is dropped then, as the zone fetcher drops it. A stream
/// outlives `cancel`, which covers the fetch up to its first bytes, as an http body outlives
/// its navigation: it stops once nobody holds it.
pub async fn run(
    protocol: &dyn Protocol,
    request: ProtocolRequest<'_>,
    cancel: &CancellationToken,
    observer: Arc<dyn NetObserver + Send + Sync>,
) -> Option<Served> {
    // A navigation cancelled while its request sat in the I/O queue must not go on to emit
    // network events for it, as for `data:`.
    if cancel.is_cancelled() {
        return None;
    }
    let url = request.url.clone();
    observer.on_event(NetEvent::Accepted {
        url: url.clone(),
        joined: false,
    });
    observer.on_event(NetEvent::Started { url: url.clone() });
    let started = std::time::Instant::now();

    let fetched = tokio::select! {
        _ = cancel.cancelled() => {
            observer.on_event(NetEvent::Cancelled { url, reason: "request cancelled" });
            return None;
        }
        fetched = protocol.fetch(request) => fetched,
    };
    let response = match fetched {
        Ok(response) => response,
        Err(ProtocolError { error, certificates }) => {
            observer.on_event(NetEvent::Failed {
                url,
                error: anyhow::anyhow!(error.clone()),
            });
            return Some(Served {
                result: FetchResult::Error(error),
                certificates,
            });
        }
    };

    let len = response.body.len() as u64;
    if let Some(rest) = response.rest {
        let headers = synthesized_headers(&response.content_type, None);
        observer.on_event(NetEvent::ResponseHeaders {
            url: url.clone(),
            status: 200,
            headers: headers.clone(),
        });
        observer.on_event(NetEvent::Progress {
            received_bytes: len,
            expected_length: None,
            elapsed: started.elapsed(),
        });
        let mut meta = FetchResultMeta::synthetic(response.final_url.unwrap_or_else(|| url.clone()));
        meta.headers = headers;
        meta.content_type = Some(response.content_type);
        meta.has_body = true;
        meta.peer_addr = response.peer_addr;
        let shared = stream_body(rest, len, url, started, observer);
        return Some(Served {
            result: FetchResult::Stream {
                meta,
                peek_buf: PeekBuf::from_vec(response.body.to_vec()),
                shared,
            },
            certificates: response.certificates,
        });
    }
    let headers = synthesized_headers(&response.content_type, Some(len));
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

    let mut meta = FetchResultMeta::synthetic(response.final_url.unwrap_or(url));
    meta.headers = headers;
    meta.content_length = Some(len);
    meta.content_type = Some(response.content_type);
    meta.has_body = len > 0;
    meta.peer_addr = response.peer_addr;
    Some(Served {
        result: FetchResult::Buffered {
            meta,
            body: response.body,
        },
        certificates: response.certificates,
    })
}

/// The `Content-Type` and `Content-Length` a protocol response is presented with; no length
/// for a body that streams.
pub fn synthesized_headers(content_type: &str, len: Option<u64>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    if let Ok(v) = content_type.parse() {
        headers.insert(http::header::CONTENT_TYPE, v);
    }
    if let Some(Ok(v)) = len.map(|len| len.to_string().parse()) {
        headers.insert(http::header::CONTENT_LENGTH, v);
    }
    headers
}

/// The body `rest` is read into, after the `received` bytes the response already holds:
/// with no cap on its size and no deadline for the whole, ending in a timeout after
/// `rest.idle` without a byte. Reading starts when the body is first subscribed to and stops
/// once nobody holds it. Its progress and its end are reported to `observer` as a fetch's.
pub fn stream_body(
    rest: BodyStream,
    received: u64,
    url: Url,
    started: std::time::Instant,
    observer: Arc<dyn NetObserver + Send + Sync>,
) -> Arc<SharedBody> {
    let reader = Observed {
        inner: rest.reader,
        received,
        url,
        started,
        observer,
    };
    SharedBody::from_reader(
        reader,
        ReaderOptions {
            buf_size: 64 * 1024,
            idle_timeout: Some(rest.idle),
            ..ReaderOptions::default()
        },
    )
}

/// The whole of a streamed body, `peek` first, or an error when it is over `cap` bytes.
pub async fn collect(peek: PeekBuf, shared: Arc<SharedBody>, cap: usize) -> Result<Bytes, NetError> {
    use tokio::io::AsyncReadExt;
    let mut body = Vec::new();
    SharedBody::combined_reader_lossless(peek, shared)
        .take(cap as u64 + 1)
        .read_to_end(&mut body)
        .await
        .map_err(NetError::from)?;
    if body.len() > cap {
        return Err(socket::invalid(format!("response is larger than {cap} bytes")));
    }
    Ok(Bytes::from(body))
}

/// A body's reader that reports what it reads: progress on each read, the end at EOF, and a
/// read that fails.
struct Observed {
    inner: Box<dyn tokio::io::AsyncRead + Send + Unpin>,
    received: u64,
    url: Url,
    started: std::time::Instant,
    observer: Arc<dyn NetObserver + Send + Sync>,
}

impl tokio::io::AsyncRead for Observed {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = &mut *self;
        let before = buf.filled().len();
        let polled = std::pin::Pin::new(&mut this.inner).poll_read(cx, buf);
        match &polled {
            std::task::Poll::Ready(Ok(())) => {
                let read = (buf.filled().len() - before) as u64;
                if read == 0 {
                    this.observer.on_event(NetEvent::Finished {
                        received_bytes: this.received,
                        elapsed: this.started.elapsed(),
                        url: this.url.clone(),
                    });
                } else {
                    this.received += read;
                    this.observer.on_event(NetEvent::Progress {
                        received_bytes: this.received,
                        expected_length: None,
                        elapsed: this.started.elapsed(),
                    });
                }
            }
            std::task::Poll::Ready(Err(e)) => this.observer.on_event(NetEvent::Failed {
                url: this.url.clone(),
                error: anyhow::anyhow!(e.to_string()),
            }),
            std::task::Poll::Pending => {}
        }
        polled
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::protocols::known_hosts::CertPin;
    use parking_lot::Mutex;

    /// Answers every URL with its own path as plain text.
    struct Echo;

    #[async_trait]
    impl Protocol for Echo {
        fn scheme(&self) -> &'static str {
            "echo"
        }
        fn setting(&self) -> &'static str {
            "net.echo.enabled"
        }
        fn default_port(&self) -> u16 {
            7
        }
        fn encrypted(&self) -> bool {
            false
        }
        async fn fetch(&self, request: ProtocolRequest<'_>) -> Result<ProtocolResponse, ProtocolError> {
            let certificates = vec![ServerPin {
                host: "host".into(),
                port: 7,
                pin: CertPin::of(b"echo"),
            }];
            // Fails after the server showed its certificate.
            if request.url.path() == "/fail" {
                return Err(ProtocolError {
                    error: NetError::Other(Arc::new(anyhow::anyhow!("asked to fail"))),
                    certificates,
                });
            }
            Ok(ProtocolResponse {
                content_type: "text/plain".into(),
                body: Bytes::from(request.url.path().to_string()),
                peer_addr: Some("192.0.2.7:70".parse().unwrap()),
                certificates,
                final_url: None,
                rest: None,
            })
        }
    }

    /// Answers with a body whose rest streams.
    struct Trickle;

    #[async_trait]
    impl Protocol for Trickle {
        fn scheme(&self) -> &'static str {
            "trickle"
        }
        fn setting(&self) -> &'static str {
            "net.trickle.enabled"
        }
        fn default_port(&self) -> u16 {
            7
        }
        fn encrypted(&self) -> bool {
            false
        }
        async fn fetch(&self, _request: ProtocolRequest<'_>) -> Result<ProtocolResponse, ProtocolError> {
            Ok(ProtocolResponse {
                content_type: "application/octet-stream".into(),
                body: Bytes::from_static(b"head-"),
                rest: Some(BodyStream {
                    reader: Box::new(&b"rest"[..]),
                    idle: std::time::Duration::from_secs(5),
                }),
                peer_addr: None,
                certificates: Vec::new(),
                final_url: None,
            })
        }
    }

    /// Never answers.
    struct Stall;

    #[async_trait]
    impl Protocol for Stall {
        fn scheme(&self) -> &'static str {
            "stall"
        }
        fn setting(&self) -> &'static str {
            "net.stall.enabled"
        }
        fn default_port(&self) -> u16 {
            7
        }
        fn encrypted(&self) -> bool {
            false
        }
        async fn fetch(&self, _request: ProtocolRequest<'_>) -> Result<ProtocolResponse, ProtocolError> {
            std::future::pending().await
        }
    }

    #[derive(Default)]
    struct Recorder(Mutex<Vec<String>>);

    impl NetObserver for Recorder {
        fn on_event(&self, event: NetEvent) {
            let name = format!("{event:?}");
            let name = name.split([' ', '{', '(']).next().unwrap_or_default().to_string();
            self.0.lock().push(name);
        }
    }

    fn request(url: &str, kind: NetResourceKind) -> FetchRequest {
        FetchRequest::builder(http::Method::GET, Url::parse(url).unwrap())
            .with_kind(kind)
            .build()
    }

    /// What the broker and then the serving side together make of `req`.
    async fn serve(
        protocol: &dyn Protocol,
        req: &FetchRequest,
        enabled: bool,
        cancel: &CancellationToken,
    ) -> (Option<FetchResult>, Vec<String>) {
        let recorder = Arc::new(Recorder::default());
        let request = ProtocolRequest {
            url: &req.url,
            reach: AddressSpace::Loopback,
            pins: None,
            identity: None,
        };
        let result = match refuse(protocol, req, enabled, &*recorder) {
            Some(refused) => Some(refused),
            None => run(protocol, request, cancel, recorder.clone())
                .await
                .map(|served| served.result),
        };
        let events = recorder.0.lock().clone();
        (result, events)
    }

    #[test]
    fn unknown_schemes_have_no_protocol() {
        assert!(lookup("http").is_none());
        assert!(lookup("echo").is_none());
    }

    #[cfg(feature = "gopher")]
    #[test]
    fn gopher_is_a_protocol() {
        let gopher = lookup("gopher").expect("gopher compiled in");
        assert_eq!(gopher.setting(), "net.gopher.enabled");
    }

    #[cfg(feature = "finger")]
    #[test]
    fn finger_is_a_protocol() {
        let finger = lookup("finger").expect("finger compiled in");
        assert_eq!(finger.setting(), "net.finger.enabled");
    }

    #[test]
    fn the_web_is_a_network_place_and_local_pages_are_not() {
        let url = |s: &str| Url::parse(s).unwrap();
        assert_eq!(transport(&url("https://a.test/")), Some(Transport::Encrypted));
        assert_eq!(transport(&url("http://a.test/")), Some(Transport::Cleartext));
        for local in [
            "file:///etc/passwd",
            "about:blank",
            "data:text/plain,x",
            "gosub://settings",
        ] {
            assert_eq!(transport(&url(local)), None, "{local}");
            assert!(!is_network_place(&url(local)), "{local}");
        }
        assert!(is_network_place(&url("http://a.test/")));
    }

    #[cfg(feature = "gopher")]
    #[test]
    fn a_gopher_page_is_a_cleartext_network_place() {
        let url = Url::parse("gopher://a.test/1/").unwrap();
        assert_eq!(transport(&url), Some(Transport::Cleartext));
        assert!(is_network_place(&url));
    }

    #[cfg(feature = "finger")]
    #[test]
    fn a_finger_page_is_a_cleartext_network_place() {
        let url = Url::parse("finger://a.test/bob").unwrap();
        assert_eq!(transport(&url), Some(Transport::Cleartext));
        assert!(is_network_place(&url));
    }

    #[tokio::test]
    async fn a_navigation_is_served_with_the_events_of_a_fetch() {
        let req = request("echo://host/hello", NetResourceKind::Primary);
        let (result, events) = serve(&Echo, &req, true, &CancellationToken::new()).await;
        let Some(FetchResult::Buffered { meta, body }) = result else {
            panic!("expected a buffered response");
        };
        assert_eq!(&body[..], b"/hello");
        assert_eq!(meta.content_type.as_deref(), Some("text/plain"));
        assert_eq!(meta.content_length, Some(6));
        assert_eq!(meta.peer_addr, Some("192.0.2.7:70".parse().unwrap()));
        assert_eq!(
            events,
            ["Accepted", "Started", "ResponseHeaders", "Progress", "Finished"]
        );
    }

    #[tokio::test]
    async fn a_body_that_streams_is_served_as_a_stream_and_finishes_when_read() {
        let url = Url::parse("trickle://host/").unwrap();
        let request = ProtocolRequest {
            url: &url,
            reach: AddressSpace::Loopback,
            pins: None,
            identity: None,
        };
        let recorder = Arc::new(Recorder::default());
        let served = run(&Trickle, request, &CancellationToken::new(), recorder.clone())
            .await
            .expect("served");
        let FetchResult::Stream { meta, peek_buf, shared } = served.result else {
            panic!("expected a stream");
        };
        assert_eq!(meta.content_length, None);
        assert!(meta.headers.get(http::header::CONTENT_LENGTH).is_none());
        assert_eq!(
            *recorder.0.lock(),
            ["Accepted", "Started", "ResponseHeaders", "Progress"]
        );

        let body = collect(peek_buf, shared, 64).await.unwrap();
        assert_eq!(&body[..], b"head-rest");
        assert_eq!(recorder.0.lock().last().map(String::as_str), Some("Finished"));
    }

    #[tokio::test]
    async fn the_certificate_a_response_showed_is_served_beside_it() {
        let url = Url::parse("echo://host/hello").unwrap();
        let request = ProtocolRequest {
            url: &url,
            reach: AddressSpace::Loopback,
            pins: None,
            identity: None,
        };
        let served = run(&Echo, request, &CancellationToken::new(), Arc::new(Recorder::default()))
            .await
            .expect("served");
        assert_eq!(served.certificates[0].pin, CertPin::of(b"echo"));
    }

    #[tokio::test]
    async fn the_certificate_a_failed_fetch_showed_is_served_beside_the_error() {
        let url = Url::parse("echo://host/fail").unwrap();
        let request = ProtocolRequest {
            url: &url,
            reach: AddressSpace::Loopback,
            pins: None,
            identity: None,
        };
        let served = run(&Echo, request, &CancellationToken::new(), Arc::new(Recorder::default()))
            .await
            .expect("served");
        assert!(matches!(served.result, FetchResult::Error(_)));
        assert_eq!(served.certificates[0].pin, CertPin::of(b"echo"));
    }

    #[tokio::test]
    async fn a_disabled_protocol_is_refused() {
        let req = request("echo://host/hello", NetResourceKind::Primary);
        let (result, events) = serve(&Echo, &req, false, &CancellationToken::new()).await;
        assert!(matches!(
            result,
            Some(FetchResult::Error(NetError::Blocked {
                reason: BlockReason::UrlPolicy,
                ..
            }))
        ));
        assert_eq!(events, ["Accepted", "Blocked"]);
    }

    #[tokio::test]
    async fn a_subresource_is_refused() {
        let req = request("echo://host/hello", NetResourceKind::Asset);
        let (result, _) = serve(&Echo, &req, true, &CancellationToken::new()).await;
        assert!(matches!(result, Some(FetchResult::Error(NetError::Blocked { .. }))));
    }

    #[tokio::test]
    async fn a_fetch_cancelled_in_the_queue_emits_nothing() {
        let req = request("stall://host/", NetResourceKind::Primary);
        let cancel = CancellationToken::new();
        cancel.cancel();
        let (result, events) = serve(&Stall, &req, true, &cancel).await;
        assert!(result.is_none());
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn a_fetch_cancelled_in_flight_drops_its_reply() {
        let req = request("stall://host/", NetResourceKind::Primary);
        let cancel = CancellationToken::new();
        let later = cancel.clone();
        tokio::spawn(async move {
            tokio::task::yield_now().await;
            later.cancel();
        });
        let (result, events) = serve(&Stall, &req, true, &cancel).await;
        assert!(result.is_none());
        assert_eq!(events, ["Accepted", "Started", "Cancelled"]);
    }
}
