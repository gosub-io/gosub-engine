//! The TCP side of an engine protocol: resolving its host, connecting, and reading a
//! response that ends when the server closes.
//!
//! What a protocol may reach is the caller's `reach`. [`resolve`] judges every address a name
//! answers, as `ssrf::StrictResolver` judges a name for http, and [`connect`] judges again
//! each address it is handed, so a connection to an address a server named (a second
//! connection, a redirect of the protocol's own) is held to the same reach as the first.
//! Ports the Fetch standard blocks are refused whatever the reach, except the protocol's own
//! port ([`port_refused`]).

use crate::net::protocols::Protocol;
use crate::net::ssrf::{classify_ip, parse_ip_literal, AddressSpace};
use crate::net::types::NetError;
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

/// Longest the whole exchange may take once connected: a protocol with no length to wait for
/// would otherwise be held forever by a server that never closes.
pub const RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);

/// Most bytes of one response. Without a length any server could fill memory. Kept under
/// what one network-process reply carries in-band.
pub const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

/// Whether `port` is refused to `protocol`: port 0, and the ports the Fetch standard blocks
/// for http, which are other services a line of text could be a command to. A protocol's own
/// port is on that list for some (finger's 79), and stays open to that protocol only.
pub fn port_refused(protocol: &dyn Protocol, port: u16) -> bool {
    port == 0 || (port != protocol.default_port() && gosub_sonar::net::port_blocking::is_bad_port(port))
}

/// The addresses `host` resolves to on `port`, or a refusal when the port is refused to
/// `protocol` or any address is beyond `reach`: which answer the OS connects to is not this
/// code's choice, so a name answering a public and a loopback address is refused whole.
pub async fn resolve(
    protocol: &dyn Protocol,
    url: &Url,
    host: &str,
    port: u16,
    reach: AddressSpace,
) -> Result<Vec<SocketAddr>, NetError> {
    if port_refused(protocol, port) {
        return Err(blocked(BlockReason::BadPort, url));
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
    Ok(addrs)
}

/// The most public of `addrs`: what a page served without a connection is placed by, so it
/// is never placed more privately than its host can be.
pub fn most_public(addrs: &[SocketAddr]) -> Option<SocketAddr> {
    let rank = |addr: &&SocketAddr| match classify_ip(addr.ip()) {
        None => 0,
        Some((AddressSpace::Local, _)) => 1,
        Some(_) => 2,
    };
    addrs.iter().min_by_key(rank).copied()
}

/// A connection to the first of `addrs` that answers, within [`CONNECT_TIMEOUT`] for all of
/// them together. An address beyond `reach` is refused before anything is sent to it,
/// wherever the address came from.
pub async fn connect(url: &Url, host: &str, addrs: &[SocketAddr], reach: AddressSpace) -> Result<TcpStream, NetError> {
    for addr in addrs {
        check_reach(*addr, reach, url, host)?;
    }
    let connect = async {
        let mut last_error = None;
        for addr in addrs {
            match TcpStream::connect(addr).await {
                Ok(s) => return Ok(s),
                Err(e) => last_error = Some(io(e)),
            }
        }
        Err(last_error.unwrap_or_else(|| invalid("no address to connect to")))
    };
    tokio::time::timeout(CONNECT_TIMEOUT, connect)
        .await
        .map_err(|_| NetError::Timeout(format!("connecting to {host}")))?
}

/// Send `line` on a connection to `addrs` and read the response to its end, which is when the
/// server closes. Returns the body and the peer it came from.
pub async fn exchange(
    url: &Url,
    host: &str,
    addrs: &[SocketAddr],
    reach: AddressSpace,
    line: &[u8],
) -> Result<(Vec<u8>, Option<SocketAddr>), NetError> {
    let mut stream = connect(url, host, addrs, reach).await?;
    let peer = stream.peer_addr().ok();

    let talk = async {
        stream.write_all(line).await?;
        let mut body = Vec::new();
        (&mut stream)
            .take(MAX_RESPONSE_BYTES as u64 + 1)
            .read_to_end(&mut body)
            .await?;
        Ok::<_, std::io::Error>(body)
    };
    let body = tokio::time::timeout(RESPONSE_TIMEOUT, talk)
        .await
        .map_err(|_| NetError::Timeout(format!("waiting for {host}")))?
        .map_err(io)?;
    if body.len() > MAX_RESPONSE_BYTES {
        return Err(invalid(format!(
            "response from {host} is larger than {MAX_RESPONSE_BYTES} bytes"
        )));
    }
    Ok((body, peer))
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
    use crate::net::protocols::ProtocolResponse;
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
        async fn fetch(&self, _url: &Url, _reach: AddressSpace) -> Result<ProtocolResponse, NetError> {
            Err(invalid("not fetched in this test"))
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
        async fn fetch(&self, _url: &Url, _reach: AddressSpace) -> Result<ProtocolResponse, NetError> {
            Err(invalid("not fetched in this test"))
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
        assert_eq!(most_public(&addrs), Some(addrs[2]));
        assert_eq!(most_public(&addrs[..2]), Some(addrs[1]));
    }

    #[tokio::test]
    async fn connect_holds_an_address_it_is_handed_to_the_reach() {
        // Not from `resolve`: an address a server named, as a second connection would use.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = Url::parse("on70://public.test/").unwrap();
        let err = connect(&url, "public.test", &[addr], AddressSpace::Public)
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
        connect(&url, "public.test", &[addr], AddressSpace::Loopback)
            .await
            .unwrap();
    }
}
