//! `gopher://` (RFC 1436; URLs per RFC 4266), one of the engine's own [protocols].
//!
//! A request is one line on a TCP connection - the selector, CRLF - and the response is
//! everything the server sends until it closes. There is no status, no header and no length,
//! so the item type in the URL is all that says what the bytes are:
//! - menus (`1`) and search results (`7`) become an HTML page of links,
//! - text files (`0`) become a `<pre>` page,
//! - HTML (`h`) is served as it is,
//! - everything else is served raw under a sniffed media type, so images display and
//!   anything unknown goes to the download offer.
//!
//! What it may reach is the caller's `reach`, judged on every address the host resolves to,
//! as `ssrf::StrictResolver` judges a name for http. Ports the Fetch standard blocks are
//! refused whatever the reach. Both matter more here than for http: a selector is an
//! arbitrary line, which is a complete command to more than one service listening on a
//! private port.
//!
//! [protocols]: crate::net::protocols

mod menu;
mod render;
mod request;

use crate::net::protocols::{Protocol, ProtocolResponse};
use crate::net::ssrf::{classify_ip, parse_ip_literal, AddressSpace};
use crate::net::types::NetError;
use async_trait::async_trait;
use bytes::Bytes;
use gosub_sonar::net::types::BlockReason;
use request::Request;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use url::Url;

/// Longest the name lookup, and then each connection attempt, may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Longest the whole exchange may take once connected: gopher has no length to wait for, so
/// a server that never closes would otherwise hold the request forever.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);

/// Most bytes of one response. Gopher sends no length, so without a cap any server could fill
/// memory. Kept under what one network-process reply carries in-band.
pub const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

/// The gopher protocol.
pub struct Gopher;

#[async_trait]
impl Protocol for Gopher {
    fn scheme(&self) -> &'static str {
        "gopher"
    }

    fn setting(&self) -> &'static str {
        "net.gopher.enabled"
    }

    async fn fetch(&self, url: &Url, reach: AddressSpace) -> Result<ProtocolResponse, NetError> {
        let request = Request::from_url(url).map_err(invalid)?;
        if request.needs_search_terms() {
            return Ok(html(render::search_page(url), None));
        }
        if request.port == 0 || gosub_sonar::net::port_blocking::is_bad_port(request.port) {
            return Err(blocked(BlockReason::BadPort, url));
        }
        let addrs = resolve(&request, reach, url).await?;
        let (body, peer) = exchange(&request, &addrs).await?;
        Ok(present(&request, url, body, peer))
    }
}

/// The addresses `request`'s host resolves to, or a refusal when any of them is beyond
/// `reach`: which answer the OS connects to is not this code's choice, so a name answering a
/// public and a loopback address is refused whole.
async fn resolve(request: &Request, reach: AddressSpace, url: &Url) -> Result<Vec<SocketAddr>, NetError> {
    // An address in any of its spellings (`2130706433`, `0x7f000001`, `127.1`) is read here,
    // not left to whatever the system resolver makes of it.
    let addrs: Vec<SocketAddr> = match parse_ip_literal(&request.host) {
        Some(ip) => vec![SocketAddr::new(ip, request.port)],
        None => {
            let lookup = tokio::net::lookup_host((request.host.as_str(), request.port));
            tokio::time::timeout(CONNECT_TIMEOUT, lookup)
                .await
                .map_err(|_| NetError::Timeout(format!("resolving {}", request.host)))?
                .map_err(io)?
                .collect()
        }
    };
    if addrs.is_empty() {
        return Err(invalid(format!("host {} did not resolve", request.host)));
    }
    for addr in &addrs {
        if let Some((space, category)) = classify_ip(addr.ip()) {
            if !reach.may_reach(space) {
                log::warn!(
                    "refusing {url}: {} resolves to {}, which is {category} (private network policy)",
                    request.host,
                    addr.ip()
                );
                return Err(blocked(BlockReason::UrlPolicy, url));
            }
        }
    }
    Ok(addrs)
}

/// Send the request line to the first address that answers and read the response to its end.
async fn exchange(request: &Request, addrs: &[SocketAddr]) -> Result<(Vec<u8>, Option<SocketAddr>), NetError> {
    let mut stream = None;
    let mut last_error = None;
    for addr in addrs {
        match tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(addr)).await {
            Ok(Ok(s)) => {
                stream = Some(s);
                break;
            }
            Ok(Err(e)) => last_error = Some(io(e)),
            Err(_) => last_error = Some(NetError::Timeout(format!("connecting to {addr}"))),
        }
    }
    let Some(mut stream) = stream else {
        return Err(last_error.unwrap_or_else(|| invalid("no address to connect to")));
    };
    let peer = stream.peer_addr().ok();

    let talk = async {
        stream.write_all(&request.wire_line()).await?;
        let mut body = Vec::new();
        (&mut stream)
            .take(MAX_RESPONSE_BYTES as u64 + 1)
            .read_to_end(&mut body)
            .await?;
        Ok::<_, std::io::Error>(body)
    };
    let body = tokio::time::timeout(RESPONSE_TIMEOUT, talk)
        .await
        .map_err(|_| NetError::Timeout(format!("waiting for {}", request.host)))?
        .map_err(io)?;
    if body.len() > MAX_RESPONSE_BYTES {
        return Err(invalid(format!(
            "gopher response from {} is larger than {MAX_RESPONSE_BYTES} bytes",
            request.host
        )));
    }
    Ok((body, peer))
}

/// What the response is, by the item type the URL asked for.
fn present(request: &Request, url: &Url, body: Vec<u8>, peer: Option<SocketAddr>) -> ProtocolResponse {
    match request.item_type {
        '1' | request::SEARCH_TYPE => html(render::menu_page(url, &body), peer),
        '3' => html(render::error_page(url, &error_text(&body)), peer),
        // A server answers a selector it does not have with an error menu, whatever type
        // was asked for.
        _ if is_error_menu(&body) => html(render::error_page(url, &error_text(&body)), peer),
        '0' => html(render::text_page(url, &body), peer),
        'h' => ProtocolResponse {
            content_type: "text/html".into(),
            body: Bytes::from(body),
            peer_addr: peer,
        },
        _ => ProtocolResponse {
            content_type: sniff(&body).into(),
            body: Bytes::from(body),
            peer_addr: peer,
        },
    }
}

/// Whether a body is a menu holding an error item (`3<message>\t<selector>\t<host>\t<port>`)
/// first. All four fields are required, so a binary file that merely starts with `3` and a
/// tab is not taken for one.
fn is_error_menu(body: &[u8]) -> bool {
    let first = menu::lines(body).next().unwrap_or_default();
    first.first() == Some(&b'3') && first.iter().filter(|&&b| b == b'\t').count() >= 3
}

/// An error body's message: its error items' display strings, or the body as text.
fn error_text(body: &[u8]) -> String {
    let messages: Vec<String> = menu::parse(body)
        .into_iter()
        .filter(|item| matches!(item.item_type, '3' | 'i'))
        .map(|item| item.display)
        .collect();
    if messages.is_empty() {
        String::from_utf8_lossy(body).into_owned()
    } else {
        messages.join("\n")
    }
}

/// A media type for a binary item, from its first bytes. Gopher's binary item types do not
/// say which format they are, and `I` covers every image there is.
fn sniff(body: &[u8]) -> &'static str {
    if body.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if body.starts_with(b"\xff\xd8\xff") {
        "image/jpeg"
    } else if body.starts_with(b"GIF87a") || body.starts_with(b"GIF89a") {
        "image/gif"
    } else if body.len() >= 12 && &body[..4] == b"RIFF" && &body[8..12] == b"WEBP" {
        "image/webp"
    } else if body.starts_with(b"BM") {
        "image/bmp"
    } else if body.starts_with(b"%PDF-") {
        "application/pdf"
    } else {
        "application/octet-stream"
    }
}

fn html(page: String, peer: Option<SocketAddr>) -> ProtocolResponse {
    ProtocolResponse {
        content_type: "text/html; charset=utf-8".into(),
        body: Bytes::from(page),
        peer_addr: peer,
    }
}

fn blocked(reason: BlockReason, url: &Url) -> NetError {
    NetError::Blocked {
        reason,
        url: url.clone(),
    }
}

fn io(e: std::io::Error) -> NetError {
    NetError::Io(Arc::new(e))
}

fn invalid(message: impl Into<String>) -> NetError {
    io(std::io::Error::new(std::io::ErrorKind::InvalidInput, message.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    /// A one-shot gopher server on loopback: answers the first connection with `response`
    /// and hands back the request line it read.
    async fn serve_once(response: &'static [u8]) -> (u16, tokio::task::JoinHandle<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut line = Vec::new();
            let mut byte = [0u8; 1];
            while socket.read_exact(&mut byte).await.is_ok() {
                line.push(byte[0]);
                if line.ends_with(b"\r\n") {
                    break;
                }
            }
            // A client that stops reading (the size cap) ends the write early.
            let _ = socket.write_all(response).await;
            line
        });
        (port, task)
    }

    async fn fetch(url: &str, reach: AddressSpace) -> Result<ProtocolResponse, NetError> {
        Gopher.fetch(&Url::parse(url).unwrap(), reach).await
    }

    #[tokio::test]
    async fn a_menu_is_fetched_and_rendered() {
        let (port, server) = serve_once(b"iHello\t\t\t\r\n1Sub\t/sub\t127.0.0.1\t70\r\n.\r\n").await;
        let response = fetch(&format!("gopher://127.0.0.1:{port}/1/top"), AddressSpace::Loopback)
            .await
            .unwrap();
        assert_eq!(server.await.unwrap(), b"/top\r\n");
        assert_eq!(response.content_type, "text/html; charset=utf-8");
        assert_eq!(response.peer_addr.map(|a| a.port()), Some(port));
        let page = std::str::from_utf8(&response.body).unwrap();
        assert!(page.contains("<a href=\"gopher://127.0.0.1/1/sub\">Sub</a>"));
    }

    #[tokio::test]
    async fn a_search_sends_its_terms_after_a_tab() {
        let (port, server) = serve_once(b"0Hit\t/hit\th\t70\r\n.\r\n").await;
        fetch(
            &format!("gopher://127.0.0.1:{port}/7/find?q=two+words"),
            AddressSpace::Loopback,
        )
        .await
        .unwrap();
        assert_eq!(server.await.unwrap(), b"/find\ttwo words\r\n");
    }

    #[tokio::test]
    async fn a_search_without_terms_is_a_form_and_no_connection() {
        // Port 1 is never contacted (it is a blocked port): the prompt needs no server.
        let response = fetch("gopher://127.0.0.1:1/7/find", AddressSpace::Public)
            .await
            .unwrap();
        assert!(std::str::from_utf8(&response.body).unwrap().contains("<form"));
        assert_eq!(response.peer_addr, None);
    }

    #[tokio::test]
    async fn a_binary_item_is_served_raw_and_sniffed() {
        let (port, _server) = serve_once(b"GIF89a\x01\x00\x01\x00").await;
        let response = fetch(&format!("gopher://127.0.0.1:{port}/I/pic"), AddressSpace::Loopback)
            .await
            .unwrap();
        assert_eq!(response.content_type, "image/gif");
        assert_eq!(&response.body[..6], b"GIF89a");
    }

    #[tokio::test]
    async fn an_error_menu_for_a_text_item_is_an_error_page() {
        let (port, _server) = serve_once(b"3'/nope' does not exist\t\terror.host\t1\r\n.\r\n").await;
        let response = fetch(&format!("gopher://127.0.0.1:{port}/0/nope"), AddressSpace::Loopback)
            .await
            .unwrap();
        let page = std::str::from_utf8(&response.body).unwrap();
        assert!(page.contains("class=\"e\">'/nope' does not exist</pre>"));
    }

    #[tokio::test]
    async fn loopback_is_refused_to_a_public_reach() {
        let (port, _server) = serve_once(b".\r\n").await;
        let err = fetch(&format!("gopher://127.0.0.1:{port}/1/"), AddressSpace::Public)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            NetError::Blocked {
                reason: BlockReason::UrlPolicy,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn an_alternate_spelling_of_loopback_is_refused_too() {
        // `2130706433` is 127.0.0.1; the resolver reads it, so the check sees the address.
        let err = fetch("gopher://2130706433:7070/1/", AddressSpace::Public)
            .await
            .unwrap_err();
        assert!(matches!(err, NetError::Blocked { .. }), "{err:?}");
    }

    #[tokio::test]
    async fn a_blocked_port_is_refused_whatever_the_reach() {
        let err = fetch("gopher://127.0.0.1:25/1/", AddressSpace::Loopback)
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
    async fn a_response_over_the_cap_fails() {
        let big: &'static [u8] = Box::leak(vec![b'x'; MAX_RESPONSE_BYTES + 1].into_boxed_slice());
        let (port, _server) = serve_once(big).await;
        let err = fetch(&format!("gopher://127.0.0.1:{port}/9/big"), AddressSpace::Loopback)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("larger than"), "{err}");
    }

    #[tokio::test]
    async fn a_selector_with_a_line_break_never_connects() {
        let err = fetch("gopher://127.0.0.1:6379/1/x%0D%0AFLUSHALL", AddressSpace::Loopback)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("forbidden byte"), "{err}");
    }

    #[test]
    fn only_a_whole_error_line_is_an_error_menu() {
        assert!(is_error_menu(b"3Not found\t\terror.host\t1\r\n"));
        assert!(!is_error_menu(b"3\t\x00\x01binary"));
        assert!(!is_error_menu(b"1Menu\t/\th\t70\r\n"));
    }

    #[test]
    fn binary_types_are_sniffed() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\nrest"), "image/png");
        assert_eq!(sniff(b"\xff\xd8\xff\xe0"), "image/jpeg");
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), "image/webp");
        assert_eq!(sniff(b"%PDF-1.7"), "application/pdf");
        assert_eq!(sniff(b"PK\x03\x04"), "application/octet-stream");
    }
}
