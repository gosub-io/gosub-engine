//! The TCP side of an engine protocol: resolving its host, connecting, and reading a
//! response that ends when the server closes.
//!
//! What a protocol may reach is the caller's `reach`. [`resolve`] judges every address a name
//! answers, as `ssrf::StrictResolver` judges a name for http, and [`connect`] judges again
//! each address it is handed, so a connection to an address a server named (a second
//! connection, a redirect of the protocol's own) is held to the same reach as the first.
//! Ports the Fetch standard blocks are refused whatever the reach, except the protocol's own
//! port ([`port_refused`]).
//!
//! A connection goes through the SOCKS proxy the environment names for it ([`socks`]). The
//! proxy then resolves the host, so the engine never sees the address a name stands for and
//! cannot hold it to the reach: where a name leads is the proxy's business, as it is for http
//! through a proxy. What can still be judged is judged before anything goes to the proxy: an
//! IP-literal host, and `localhost` with its subdomains (RFC 6761), taken as loopback. Other
//! names that lead to this machine or the local network (`/etc/hosts` aliases, the proxy's own
//! network) are not caught. Blocked ports are refused as for a direct connection. The proxy's
//! own address is the user's choice and is not held to the reach: it is typically loopback (a
//! local Tor on 127.0.0.1:9050). A proxied response has no peer to place it by
//! ([`ssrf::space_of_response`]).
//!
//! [`ssrf::space_of_response`]: crate::net::ssrf::space_of_response

use crate::net::protocols::socks::{self, Proxies, Proxy};
use crate::net::protocols::{BodyStream, Protocol};
use crate::net::ssrf::{classify_ip, parse_ip_literal, AddressSpace};
use crate::net::types::NetError;
use cow_utils::CowUtils;
use gosub_sonar::net::types::BlockReason;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use url::Url;

/// Longest the name lookup may take, and then connecting, across every address the name
/// has. With [`RESPONSE_TIMEOUT`] this stays well inside the broker's wait for a
/// network-process reply (`client::REPLY_TIMEOUT`, 120 s), so a slow server ends in the
/// protocol's own error rather than the broker giving up on the child.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Longest the whole exchange may take once connected, for a response read whole: a protocol
/// with no length to wait for would otherwise be held forever by a server that never closes.
pub const RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);

/// Most bytes of a response read whole. Without a length any server could fill memory. Kept
/// under what one network-process reply carries in-band.
pub const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

/// Longest a streamed response may go without a byte. Under the broker's patience with a
/// silent body stream (`client::RING_IDLE_TIMEOUT`, 60 s), so a stalled server ends in the
/// protocol's own error.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// Bytes [`open`] reads before handing a response on: enough to sniff a media type and to see
/// a whole first menu line.
pub const PEEK_BYTES: usize = 1024;

/// What a response is read under.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// The whole exchange, for a response read whole.
    pub response: Duration,
    /// Between bytes, for a response streamed.
    pub idle: Duration,
    /// Most bytes of a response read whole.
    pub max_bytes: usize,
}

impl Limits {
    pub const DEFAULT: Limits = Limits {
        response: RESPONSE_TIMEOUT,
        idle: IDLE_TIMEOUT,
        max_bytes: MAX_RESPONSE_BYTES,
    };
}

/// Whether `port` is refused to `protocol`: port 0, and the ports the Fetch standard blocks
/// for http, which are other services a line of text could be a command to. A protocol's own
/// port is on that list for some (finger's 79), and stays open to that protocol only.
pub fn port_refused(protocol: &dyn Protocol, port: u16) -> bool {
    port == 0 || (port != protocol.default_port() && gosub_sonar::net::port_blocking::is_bad_port(port))
}

/// Where a connection goes.
#[derive(Debug, Clone)]
pub enum Destination {
    /// Straight to the host, at these addresses.
    Direct(Vec<SocketAddr>),
    /// Through a SOCKS proxy, which resolves `host`.
    Proxied { proxy: Proxy, host: String, port: u16 },
}

/// Where a connection to `host` on `port` goes, or a refusal when the port is refused to
/// `protocol` or the host is beyond `reach`. Through the environment's SOCKS proxy when it
/// names one for the host; directly otherwise, to every address `host` resolves to, each
/// within `reach`: which answer the OS connects to is not this code's choice, so a name
/// answering a public and a loopback address is refused whole.
pub async fn resolve(
    protocol: &dyn Protocol,
    url: &Url,
    host: &str,
    port: u16,
    reach: AddressSpace,
) -> Result<Destination, NetError> {
    resolve_with(protocol, url, host, port, reach, &socks::FROM_ENV).await
}

/// [`resolve`] with `proxies` in place of the environment's.
pub async fn resolve_with(
    protocol: &dyn Protocol,
    url: &Url,
    host: &str,
    port: u16,
    reach: AddressSpace,
    proxies: &Proxies,
) -> Result<Destination, NetError> {
    if port_refused(protocol, port) {
        return Err(blocked(BlockReason::BadPort, url));
    }
    if let Some(proxy) = proxies.for_target(host, port).map_err(invalid)? {
        check_proxied(host, reach, url)?;
        return Ok(Destination::Proxied {
            proxy: proxy.clone(),
            host: host.to_string(),
            port,
        });
    }
    // An address in any of its spellings (`2130706433`, `0x7f000001`, `127.1`) is read here,
    // not left to whatever the system resolver makes of it.
    let addrs: Vec<SocketAddr> = match parse_ip_literal(host) {
        Some(ip) => vec![SocketAddr::new(ip, port)],
        None => {
            let lookup = tokio::net::lookup_host((host, port));
            tokio::time::timeout(CONNECT_TIMEOUT, lookup)
                .await
                .map_err(|_| NetError::Timeout(format!("resolving {host}")))?
                .map_err(io)?
                .collect()
        }
    };
    if addrs.is_empty() {
        return Err(invalid(format!("host {host} did not resolve")));
    }
    for addr in &addrs {
        check_reach(*addr, reach, url, host)?;
    }
    Ok(Destination::Direct(addrs))
}

/// The most public address of `destination`: what a page served without a connection is
/// placed by, so it is never placed more privately than its host can be. `None` through a
/// proxy, which places the page as a proxied response is.
pub fn most_public(destination: &Destination) -> Option<SocketAddr> {
    let Destination::Direct(addrs) = destination else {
        return None;
    };
    let rank = |addr: &&SocketAddr| match classify_ip(addr.ip()) {
        None => 0,
        Some((AddressSpace::Local, _)) => 1,
        Some(_) => 2,
    };
    addrs.iter().min_by_key(rank).copied()
}

/// A connection to `destination` and the peer it reached, within [`CONNECT_TIMEOUT`]. Direct,
/// to the first address that answers, the timeout for all of them together; an address beyond
/// `reach` is refused before anything is sent to it, wherever the address came from. Through a
/// proxy, the timeout covers the proxy's handshake too, and there is no peer: the one the
/// socket has is the proxy.
pub async fn connect(
    url: &Url,
    host: &str,
    destination: &Destination,
    reach: AddressSpace,
) -> Result<(TcpStream, Option<SocketAddr>), NetError> {
    connect_within(url, host, destination, reach, CONNECT_TIMEOUT).await
}

async fn connect_within(
    url: &Url,
    host: &str,
    destination: &Destination,
    reach: AddressSpace,
    timeout: Duration,
) -> Result<(TcpStream, Option<SocketAddr>), NetError> {
    let addrs = match destination {
        Destination::Direct(addrs) => addrs,
        Destination::Proxied {
            proxy,
            host: target,
            port,
        } => {
            check_proxied(target, reach, url)?;
            let stream = tokio::time::timeout(timeout, socks::connect(proxy, target, *port))
                .await
                .map_err(|_| NetError::Timeout(format!("connecting to {host} through the proxy {proxy}")))?
                .map_err(io)?;
            return Ok((stream, None));
        }
    };
    for addr in addrs {
        check_reach(*addr, reach, url, host)?;
    }
    let connect = async {
        let mut last_error = None;
        for addr in addrs {
            match TcpStream::connect(addr).await {
                Ok(s) => {
                    let peer = s.peer_addr().ok();
                    return Ok((s, peer));
                }
                Err(e) => last_error = Some(io(e)),
            }
        }
        Err(last_error.unwrap_or_else(|| invalid("no address to connect to")))
    };
    tokio::time::timeout(timeout, connect)
        .await
        .map_err(|_| NetError::Timeout(format!("connecting to {host}")))?
}

/// Send `line` on a connection to `destination` and read the response to its end, which is
/// when the server closes. Returns the body and the peer it came from.
pub async fn exchange(
    url: &Url,
    host: &str,
    destination: &Destination,
    reach: AddressSpace,
    line: &[u8],
) -> Result<(Vec<u8>, Option<SocketAddr>), NetError> {
    exchange_within(url, host, destination, reach, line, &Limits::DEFAULT).await
}

/// [`exchange`] under `limits`.
pub async fn exchange_within(
    url: &Url,
    host: &str,
    destination: &Destination,
    reach: AddressSpace,
    line: &[u8],
    limits: &Limits,
) -> Result<(Vec<u8>, Option<SocketAddr>), NetError> {
    let (mut stream, peer) = connect(url, host, destination, reach).await?;
    let body = talk_within(&mut stream, host, line, Close::Clean, limits).await?;
    Ok((body, peer))
}

/// A response [`open`] began to read.
#[derive(Debug)]
pub struct Opened {
    /// Its first bytes: [`PEEK_BYTES`] of them, or all there were.
    pub peek: Vec<u8>,
    /// The rest, still arriving. `None` when the server closed within the peek.
    pub rest: Option<BodyStream>,
    pub peer: Option<SocketAddr>,
}

/// Send `line` on a connection to `destination` and read the first bytes of the response, leaving
/// the rest to arrive as it is read: no cap on its size and no deadline for the whole, only
/// `limits.idle` between bytes, the peek included.
pub async fn open(
    url: &Url,
    host: &str,
    destination: &Destination,
    reach: AddressSpace,
    line: &[u8],
    limits: &Limits,
) -> Result<Opened, NetError> {
    let (stream, peer) = connect(url, host, destination, reach).await?;
    open_on(stream, peer, host, line, limits).await
}

/// [`open`] on a connection already made to `peer`.
pub async fn open_on(
    mut stream: TcpStream,
    peer: Option<SocketAddr>,
    host: &str,
    line: &[u8],
    limits: &Limits,
) -> Result<Opened, NetError> {
    let waiting = || NetError::Timeout(format!("waiting for {host}"));
    let send = async {
        stream.write_all(line).await?;
        stream.flush().await
    };
    tokio::time::timeout(limits.idle, send)
        .await
        .map_err(|_| waiting())?
        .map_err(io)?;
    let mut peek = vec![0u8; PEEK_BYTES];
    let mut len = 0;
    while len < PEEK_BYTES {
        let n = tokio::time::timeout(limits.idle, stream.read(&mut peek[len..]))
            .await
            .map_err(|_| waiting())?
            .map_err(io)?;
        if n == 0 {
            peek.truncate(len);
            return Ok(Opened { peek, rest: None, peer });
        }
        len += n;
    }
    let rest = BodyStream {
        reader: Box::new(stream),
        idle: limits.idle,
    };
    Ok(Opened {
        peek,
        rest: Some(rest),
        peer,
    })
}

/// The whole of a response [`open`] began: `peek` and then `rest` to its end, within
/// `limits.response` and at most `limits.max_bytes` in all.
pub async fn read_rest(mut peek: Vec<u8>, rest: BodyStream, host: &str, limits: &Limits) -> Result<Vec<u8>, NetError> {
    let max = limits.max_bytes as u64 + 1;
    let mut reader = rest.reader.take(max.saturating_sub(peek.len() as u64));
    tokio::time::timeout(limits.response, reader.read_to_end(&mut peek))
        .await
        .map_err(|_| NetError::Timeout(format!("waiting for {host}")))?
        .map_err(io)?;
    capped(peek, host, limits)
}

/// How a response may end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Close {
    /// Only by the server closing the stream.
    Clean,
    /// Also by the connection dropping where a clean end was owed, which a TLS stream reads
    /// as an error: many servers close without TLS's `close_notify`. A response cut short
    /// then reads as complete.
    Unclean,
}

/// Send `line` on `stream` and read the response to its end, within [`RESPONSE_TIMEOUT`] and
/// at most [`MAX_RESPONSE_BYTES`].
pub async fn talk<S>(stream: &mut S, host: &str, line: &[u8], close: Close) -> Result<Vec<u8>, NetError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    talk_within(stream, host, line, close, &Limits::DEFAULT).await
}

/// [`talk`] under `limits`.
pub async fn talk_within<S>(
    stream: &mut S,
    host: &str,
    line: &[u8],
    close: Close,
    limits: &Limits,
) -> Result<Vec<u8>, NetError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let talk = async {
        stream.write_all(line).await?;
        stream.flush().await?;
        let mut body = Vec::new();
        let read = (&mut *stream)
            .take(limits.max_bytes as u64 + 1)
            .read_to_end(&mut body)
            .await;
        match read {
            Ok(_) => Ok(body),
            Err(e) if close == Close::Unclean && e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(body),
            Err(e) => Err(e),
        }
    };
    let body = tokio::time::timeout(limits.response, talk)
        .await
        .map_err(|_| NetError::Timeout(format!("waiting for {host}")))?
        .map_err(io)?;
    capped(body, host, limits)
}

/// `body`, or a refusal when it is over `limits.max_bytes`.
fn capped(body: Vec<u8>, host: &str, limits: &Limits) -> Result<Vec<u8>, NetError> {
    if body.len() > limits.max_bytes {
        return Err(invalid(format!(
            "response from {host} is larger than {} bytes",
            limits.max_bytes
        )));
    }
    Ok(body)
}

/// Refuse a host the proxy is to resolve when it is plainly beyond `reach`: an IP literal in a
/// space `reach` does not include, or `localhost` and its subdomains unless `reach` is
/// loopback. Any other name passes; the proxy decides where it leads.
fn check_proxied(host: &str, reach: AddressSpace, url: &Url) -> Result<(), NetError> {
    if let Some(ip) = parse_ip_literal(host) {
        return check_reach(SocketAddr::new(ip, 0), reach, url, host);
    }
    let name = host.trim_end_matches('.').cow_to_ascii_lowercase();
    if (name == "localhost" || name.ends_with(".localhost")) && !reach.may_reach(AddressSpace::Loopback) {
        log::warn!("refusing {url}: {host} is loopback (private network policy)");
        return Err(blocked(BlockReason::UrlPolicy, url));
    }
    Ok(())
}

/// Refuse `addr` when it is in a space `reach` does not include.
fn check_reach(addr: SocketAddr, reach: AddressSpace, url: &Url, host: &str) -> Result<(), NetError> {
    if let Some((space, category)) = classify_ip(addr.ip()) {
        if !reach.may_reach(space) {
            log::warn!(
                "refusing {url}: {host} is at {}, which is {category} (private network policy)",
                addr.ip()
            );
            return Err(blocked(BlockReason::UrlPolicy, url));
        }
    }
    Ok(())
}

pub fn blocked(reason: BlockReason, url: &Url) -> NetError {
    NetError::Blocked {
        reason,
        url: url.clone(),
    }
}

pub fn io(e: std::io::Error) -> NetError {
    NetError::Io(Arc::new(e))
}

pub fn invalid(message: impl Into<String>) -> NetError {
    io(std::io::Error::new(std::io::ErrorKind::InvalidInput, message.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::protocols::{ProtocolError, ProtocolRequest, ProtocolResponse};
    use async_trait::async_trait;

    /// A protocol whose own port is 79, which the Fetch standard blocks.
    struct On79;

    #[async_trait]
    impl Protocol for On79 {
        fn scheme(&self) -> &'static str {
            "on79"
        }
        fn setting(&self) -> &'static str {
            "net.on79.enabled"
        }
        fn default_port(&self) -> u16 {
            79
        }
        fn encrypted(&self) -> bool {
            false
        }
        async fn fetch(&self, _request: ProtocolRequest<'_>) -> Result<ProtocolResponse, ProtocolError> {
            Err(invalid("not fetched in this test").into())
        }
    }

    /// Same, on 70, which the standard does not block.
    struct On70;

    #[async_trait]
    impl Protocol for On70 {
        fn scheme(&self) -> &'static str {
            "on70"
        }
        fn setting(&self) -> &'static str {
            "net.on70.enabled"
        }
        fn default_port(&self) -> u16 {
            70
        }
        fn encrypted(&self) -> bool {
            false
        }
        async fn fetch(&self, _request: ProtocolRequest<'_>) -> Result<ProtocolResponse, ProtocolError> {
            Err(invalid("not fetched in this test").into())
        }
    }

    #[test]
    fn a_blocked_port_is_open_only_to_the_protocol_it_belongs_to() {
        assert!(!port_refused(&On79, 79));
        assert!(port_refused(&On70, 79));
        assert!(port_refused(&On79, 25));
        assert!(port_refused(&On70, 25));
        assert!(!port_refused(&On70, 70));
        assert!(!port_refused(&On79, 7070));
        assert!(port_refused(&On79, 0));
    }

    #[tokio::test]
    async fn a_refused_port_is_refused_before_resolving() {
        let url = Url::parse("on70://unresolvable.invalid:25/").unwrap();
        let err = resolve(&On70, &url, "unresolvable.invalid", 25, AddressSpace::Loopback)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            NetError::Blocked {
                reason: BlockReason::BadPort,
                ..
            }
        ));
    }

    #[test]
    fn a_page_without_a_connection_is_placed_by_its_most_public_address() {
        let addrs: Vec<SocketAddr> = ["127.0.0.1:70", "10.0.0.1:70", "93.184.215.14:70"]
            .iter()
            .map(|a| a.parse().unwrap())
            .collect();
        assert_eq!(most_public(&Destination::Direct(addrs.clone())), Some(addrs[2]));
        assert_eq!(most_public(&Destination::Direct(addrs[..2].to_vec())), Some(addrs[1]));
    }

    #[tokio::test]
    async fn connect_holds_an_address_it_is_handed_to_the_reach() {
        // Not from `resolve`: an address a server named, as a second connection would use.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = Url::parse("on70://public.test/").unwrap();
        let err = connect(
            &url,
            "public.test",
            &Destination::Direct(vec![addr]),
            AddressSpace::Public,
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err,
            NetError::Blocked {
                reason: BlockReason::UrlPolicy,
                ..
            }
        ));
        let accepted = tokio::time::timeout(Duration::from_millis(200), listener.accept()).await;
        assert!(accepted.is_err(), "the listener was connected to");

        // Within reach it connects.
        connect(
            &url,
            "public.test",
            &Destination::Direct(vec![addr]),
            AddressSpace::Loopback,
        )
        .await
        .unwrap();
    }

    fn through(proxy_port: u16, no_proxy: &str) -> Proxies {
        let vars = [
            ("ALL_PROXY", format!("socks5h://127.0.0.1:{proxy_port}")),
            ("NO_PROXY", no_proxy.to_string()),
        ];
        Proxies::from_env(|name| vars.iter().find(|(k, _)| *k == name).map(|(_, v)| v.clone()))
    }

    fn refused_by_policy(err: &NetError) -> bool {
        matches!(
            err,
            NetError::Blocked {
                reason: BlockReason::UrlPolicy,
                ..
            }
        )
    }

    /// A server on loopback answering one connection with `reply` after the request line.
    async fn server(reply: &'static [u8]) -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 64];
            let _ = s.read(&mut buf).await;
            let _ = s.write_all(reply).await;
        });
        port
    }

    #[tokio::test]
    async fn a_name_goes_to_the_proxy_unresolved_and_the_response_has_no_peer() {
        let origin = server(b"through").await;
        let (proxy, asked) = socks::testing::proxy(socks::testing::Behaviour::Open, Some(origin)).await;
        // `.invalid` never resolves: only the proxy can have looked it up.
        let url = Url::parse("on70://gopher.invalid/").unwrap();
        let destination = resolve_with(
            &On70,
            &url,
            "gopher.invalid",
            70,
            AddressSpace::Public,
            &through(proxy, ""),
        )
        .await
        .unwrap();
        assert!(most_public(&destination).is_none());
        // The proxy on loopback is reached from a public reach: it is the user's.
        let (body, peer) = exchange(&url, "gopher.invalid", &destination, AddressSpace::Public, b"\r\n")
            .await
            .unwrap();
        assert_eq!(body, b"through");
        assert_eq!(peer, None);
        let asked = asked.await.unwrap().unwrap();
        assert_eq!((asked.kind, asked.host.as_str(), asked.port), (3, "gopher.invalid", 70));
    }

    #[tokio::test]
    async fn a_streamed_response_through_the_proxy_has_no_peer() {
        let origin = server(b"streamed").await;
        let (proxy, _) = socks::testing::proxy(socks::testing::Behaviour::Open, Some(origin)).await;
        let url = Url::parse("on70://gopher.invalid/").unwrap();
        let destination = resolve_with(
            &On70,
            &url,
            "gopher.invalid",
            70,
            AddressSpace::Public,
            &through(proxy, ""),
        )
        .await
        .unwrap();
        let opened = open(
            &url,
            "gopher.invalid",
            &destination,
            AddressSpace::Public,
            b"\r\n",
            &Limits::DEFAULT,
        )
        .await
        .unwrap();
        assert_eq!(opened.peek, b"streamed");
        assert_eq!(opened.peer, None);
    }

    #[tokio::test]
    async fn an_address_beyond_the_reach_is_refused_before_the_proxy() {
        let (proxy, asked) = socks::testing::proxy(socks::testing::Behaviour::Open, None).await;
        let proxies = through(proxy, "");
        for host in [
            "10.0.0.1",
            "127.0.0.1",
            "0x7f000001",
            "::1",
            "localhost",
            "LocalHost.",
            "a.localhost",
        ] {
            let url = Url::parse("on70://placeholder.test/").unwrap();
            let err = resolve_with(&On70, &url, host, 70, AddressSpace::Public, &proxies)
                .await
                .unwrap_err();
            assert!(refused_by_policy(&err), "{host}: {err:?}");
            // A destination built elsewhere is held to the reach as well.
            let destination = Destination::Proxied {
                proxy: proxies.for_target(host, 70).unwrap().unwrap().clone(),
                host: host.to_string(),
                port: 70,
            };
            let err = connect(&url, host, &destination, AddressSpace::Public)
                .await
                .unwrap_err();
            assert!(refused_by_policy(&err), "{host}: {err:?}");
        }
        // Local reaches the local network, not loopback.
        let url = Url::parse("on70://localhost/").unwrap();
        let err = resolve_with(&On70, &url, "localhost", 70, AddressSpace::Local, &proxies)
            .await
            .unwrap_err();
        assert!(refused_by_policy(&err));
        resolve_with(&On70, &url, "10.0.0.1", 70, AddressSpace::Local, &proxies)
            .await
            .unwrap();
        resolve_with(&On70, &url, "localhost", 70, AddressSpace::Loopback, &proxies)
            .await
            .unwrap();
        let reached = tokio::time::timeout(Duration::from_millis(200), asked).await;
        assert!(reached.is_err(), "the proxy was connected to");
    }

    #[tokio::test]
    async fn a_blocked_port_is_refused_through_the_proxy_too() {
        let url = Url::parse("on70://example.org:25/").unwrap();
        let err = resolve_with(&On70, &url, "example.org", 25, AddressSpace::Loopback, &through(9, ""))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            NetError::Blocked {
                reason: BlockReason::BadPort,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn a_host_in_no_proxy_connects_directly() {
        let url = Url::parse("on70://127.0.0.1/").unwrap();
        let destination = resolve_with(
            &On70,
            &url,
            "127.0.0.1",
            70,
            AddressSpace::Loopback,
            &through(9, "127.0.0.0/8"),
        )
        .await
        .unwrap();
        assert!(matches!(destination, Destination::Direct(ref addrs) if addrs.len() == 1));
    }

    #[tokio::test]
    async fn an_http_proxy_leaves_the_connection_direct() {
        let origin = server(b"direct").await;
        let vars = [
            ("ALL_PROXY", "http://127.0.0.1:9"),
            ("HTTP_PROXY", "socks5h://127.0.0.1:9"),
        ];
        let proxies = Proxies::from_env(|name| vars.iter().find(|(k, _)| *k == name).map(|(_, v)| v.to_string()));
        let url = Url::parse(&format!("on70://127.0.0.1:{origin}/")).unwrap();
        let destination = resolve_with(&On70, &url, "127.0.0.1", origin, AddressSpace::Loopback, &proxies)
            .await
            .unwrap();
        let (body, peer) = exchange(&url, "127.0.0.1", &destination, AddressSpace::Loopback, b"\r\n")
            .await
            .unwrap();
        assert_eq!(body, b"direct");
        assert_eq!(peer.map(|p| p.port()), Some(origin));
    }

    #[tokio::test]
    async fn a_proxy_that_refuses_ends_in_a_clean_error() {
        let (proxy, _) = socks::testing::proxy(socks::testing::Behaviour::Refuse, None).await;
        let url = Url::parse("on70://gopher.invalid/").unwrap();
        let destination = resolve_with(
            &On70,
            &url,
            "gopher.invalid",
            70,
            AddressSpace::Public,
            &through(proxy, ""),
        )
        .await
        .unwrap();
        let err = exchange(&url, "gopher.invalid", &destination, AddressSpace::Public, b"\r\n")
            .await
            .unwrap_err();
        let NetError::Io(e) = err else {
            panic!("{err:?}");
        };
        assert_eq!(e.kind(), std::io::ErrorKind::ConnectionRefused);
    }

    #[tokio::test]
    async fn the_connect_timeout_covers_the_proxy_handshake() {
        let (proxy, _) = socks::testing::proxy(socks::testing::Behaviour::Silent, None).await;
        let url = Url::parse("on70://gopher.invalid/").unwrap();
        let destination = resolve_with(
            &On70,
            &url,
            "gopher.invalid",
            70,
            AddressSpace::Public,
            &through(proxy, ""),
        )
        .await
        .unwrap();
        let err = connect_within(
            &url,
            "gopher.invalid",
            &destination,
            AddressSpace::Public,
            Duration::from_millis(200),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, NetError::Timeout(_)), "{err:?}");
    }

    #[tokio::test]
    async fn an_unusable_proxy_fails_the_fetch_before_any_connection() {
        let origin = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = origin.local_addr().unwrap().port();
        let vars = [("ALL_PROXY", "ftp://alice:hunter2@127.0.0.1:21")];
        let proxies = Proxies::from_env(|name| vars.iter().find(|(k, _)| *k == name).map(|(_, v)| v.to_string()));
        let url = Url::parse(&format!("on70://127.0.0.1:{port}/")).unwrap();
        let err = resolve_with(&On70, &url, "127.0.0.1", port, AddressSpace::Loopback, &proxies)
            .await
            .unwrap_err();
        let NetError::Io(e) = err else {
            panic!("{err:?}");
        };
        assert_eq!(e.kind(), std::io::ErrorKind::InvalidInput);
        let message = e.to_string();
        assert!(message.contains("ftp://alice:...@127.0.0.1:21"), "{message}");
        assert!(!message.contains("hunter2"), "{message}");
        let accepted = tokio::time::timeout(Duration::from_millis(200), origin.accept()).await;
        assert!(accepted.is_err(), "the origin was connected to directly");
    }
}
