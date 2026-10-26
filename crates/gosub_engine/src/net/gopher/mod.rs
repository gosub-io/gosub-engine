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
//! The connection is [`socket`]'s: the caller's `reach` is judged on every address the host
//! resolves to, and ports the Fetch standard blocks are refused whatever the reach. Both
//! matter more here than for http: a selector is an arbitrary line, which is a complete
//! command to more than one service listening on a private port.
//!
//! [protocols]: crate::net::protocols

mod menu;
mod render;
mod request;

use crate::net::protocols::socket::{self, invalid};
use crate::net::protocols::{Protocol, ProtocolResponse};
use crate::net::ssrf::AddressSpace;
use crate::net::types::NetError;
use async_trait::async_trait;
use bytes::Bytes;
use request::Request;
use std::net::SocketAddr;
use url::Url;

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

    fn default_port(&self) -> u16 {
        request::DEFAULT_PORT
    }

    fn encrypted(&self) -> bool {
        false
    }

    async fn fetch(&self, url: &Url, reach: AddressSpace) -> Result<ProtocolResponse, NetError> {
        let request = Request::from_url(url).map_err(invalid)?;
        let addrs = socket::resolve(self, url, &request.host, request.port, reach).await?;
        if request.needs_search_terms() {
            // The prompt needs no server, but it is placed by where its server is: the form
            // submits from it, and that search may reach only as far as the prompt's space.
            return Ok(html(render::search_page(url), socket::most_public(&addrs)));
        }
        let (body, peer) = socket::exchange(url, &request.host, &addrs, reach, &request.wire_line()).await?;
        Ok(present(&request, url, body, peer))
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::protocols::socket::MAX_RESPONSE_BYTES;
    use gosub_sonar::net::types::BlockReason;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
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
        // Nothing listens: the prompt needs no server, only where it is.
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };
        let response = fetch(&format!("gopher://127.0.0.1:{port}/7/find"), AddressSpace::Loopback)
            .await
            .unwrap();
        assert!(std::str::from_utf8(&response.body).unwrap().contains("<form"));
        assert_eq!(response.peer_addr, Some(SocketAddr::from(([127, 0, 0, 1], port))));
    }

    #[tokio::test]
    async fn a_search_prompt_is_out_of_reach_like_its_server() {
        let err = fetch("gopher://127.0.0.1:7070/7/find", AddressSpace::Public)
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
    async fn finger_s_port_is_not_gopher_s() {
        // 79 is open to finger, whose port it is, and blocked to every other protocol.
        let err = fetch("gopher://127.0.0.1:79/1/", AddressSpace::Loopback)
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
