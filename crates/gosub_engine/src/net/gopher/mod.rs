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
//! Every body but an error item's streams, with no deadline for the whole, only one between
//! bytes. A raw body has no size limit: RFC 1436 reads it until the server closes and gives
//! it none. A menu or text page is served a line at a time as it arrives ([`stream`]); it is
//! held in memory to be parsed, so its body keeps the socket's cap.
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
mod stream;

use crate::net::protocols::socket::{self, invalid, Limits, Opened};
use crate::net::protocols::{BodyStream, Protocol, ProtocolError, ProtocolRequest, ProtocolResponse};
use crate::net::types::NetError;
use async_trait::async_trait;
use bytes::Bytes;
use render::Lines;
use request::Request;
use std::net::SocketAddr;
use stream::PageStream;
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

    async fn fetch(&self, request: ProtocolRequest<'_>) -> Result<ProtocolResponse, ProtocolError> {
        Ok(fetch_within(request, &Limits::DEFAULT).await?)
    }
}

/// [`Gopher::fetch`] under `limits`.
async fn fetch_within(request: ProtocolRequest<'_>, limits: &Limits) -> Result<ProtocolResponse, NetError> {
    let (url, reach) = (request.url, request.reach);
    let request = Request::from_url(url).map_err(invalid)?;
    let addrs = socket::resolve(&Gopher, url, &request.host, request.port, reach).await?;
    if request.needs_search_terms() {
        // The prompt needs no server, but it is placed by where its server is: the form
        // submits from it, and that search may reach only as far as the prompt's space.
        return Ok(html(render::search_page(url), socket::most_public(&addrs)));
    }
    let line = request.wire_line();
    if request.item_type == '3' {
        let (body, peer) = socket::exchange_within(url, &request.host, &addrs, reach, &line, limits).await?;
        return Ok(present(&request, url, body, peer));
    }
    let Opened { peek, rest, peer } = socket::open(url, &request.host, &addrs, reach, &line, limits).await?;
    let Some(rest) = rest else {
        return Ok(present(&request, url, peek, peer));
    };
    if is_error_menu(&peek) {
        let body = socket::read_rest(peek, rest, &request.host, limits).await?;
        return Ok(present(&request, url, body, peer));
    }
    let encoding = stream::encoding_for(&peek);
    let lines = match request.item_type {
        '1' | request::SEARCH_TYPE => Some(Lines::menu(url, encoding)),
        '0' => Some(Lines::text(encoding)),
        _ => None,
    };
    let (content_type, body, reader): (&str, _, Box<dyn tokio::io::AsyncRead + Send + Unpin>) = match lines {
        Some(lines) => {
            let page = PageStream::new(lines, peek, rest.reader, &request.host, limits.max_bytes);
            (
                "text/html; charset=utf-8",
                render::page_head(url).into_bytes(),
                Box::new(page),
            )
        }
        None if request.item_type == 'h' => ("text/html", peek, rest.reader),
        None => (sniff(&peek), peek, rest.reader),
    };
    Ok(ProtocolResponse {
        content_type: content_type.into(),
        body: Bytes::from(body),
        rest: Some(BodyStream {
            reader,
            idle: rest.idle,
        }),
        peer_addr: peer,
        certificates: Vec::new(),
        final_url: None,
    })
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
            rest: None,
            peer_addr: peer,
            certificates: Vec::new(),
            final_url: None,
        },
        _ => ProtocolResponse {
            content_type: sniff(&body).into(),
            body: Bytes::from(body),
            rest: None,
            peer_addr: peer,
            certificates: Vec::new(),
            final_url: None,
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
        rest: None,
        peer_addr: peer,
        certificates: Vec::new(),
        final_url: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::protocols::{self, socket::MAX_RESPONSE_BYTES};
    use crate::net::ssrf::AddressSpace;
    use crate::net::types::NetError;
    use gosub_sonar::net::types::BlockReason;
    use gosub_sonar::types::PeekBuf;
    use std::sync::Arc;
    use std::time::Duration;
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
        let url = Url::parse(url).unwrap();
        Gopher
            .fetch(ProtocolRequest {
                url: &url,
                reach,
                pins: None,
                identity: None,
            })
            .await
            .map_err(|e| e.error)
    }

    async fn fetch_under(url: &str, limits: &Limits) -> Result<ProtocolResponse, NetError> {
        let url = Url::parse(url).unwrap();
        let request = ProtocolRequest {
            url: &url,
            reach: AddressSpace::Loopback,
            pins: None,
            identity: None,
        };
        fetch_within(request, limits).await
    }

    /// The whole body of a response, read through the stream it is served as.
    async fn whole_body(response: ProtocolResponse) -> Result<Bytes, NetError> {
        let Some(rest) = response.rest else {
            return Ok(response.body);
        };
        let url = Url::parse("gopher://test/").unwrap();
        let observer = Arc::new(gosub_sonar::net::null_emitter::NullEmitter);
        let shared = protocols::stream_body(rest, 0, url, std::time::Instant::now(), observer);
        protocols::collect(PeekBuf::from_vec(response.body.to_vec()), shared, usize::MAX - 1).await
    }

    /// A gopher server on loopback that answers the first connection with `chunks`, `pause`
    /// apart, and then holds the connection open for `hold`.
    async fn serve_slowly(chunks: Vec<Vec<u8>>, pause: Duration, hold: Duration) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut line = [0u8; 256];
            let _ = socket.read(&mut line).await;
            for chunk in chunks {
                if socket.write_all(&chunk).await.is_err() {
                    return;
                }
                tokio::time::sleep(pause).await;
            }
            tokio::time::sleep(hold).await;
        });
        port
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
    async fn a_menu_over_the_cap_fails() {
        let mut big = b"iFirst\t\t\t\r\n".to_vec();
        big.resize(MAX_RESPONSE_BYTES + 1, b'x');
        let big: &'static [u8] = Box::leak(big.into_boxed_slice());
        let (port, _server) = serve_once(big).await;
        let response = fetch(&format!("gopher://127.0.0.1:{port}/1/big"), AddressSpace::Loopback)
            .await
            .unwrap();
        // The page has begun by the time the body is over the cap: it ends in the error.
        let err = whole_body(response).await.unwrap_err();
        assert!(err.to_string().contains("larger than"), "{err}");
    }

    #[tokio::test]
    async fn a_menu_under_the_cap_streams_in_full() {
        let line = b"1Sub\t/sub\t127.0.0.1\t70\r\n";
        let mut menu = line.repeat((MAX_RESPONSE_BYTES - 3) / line.len());
        menu.extend(b".\r\n");
        let menu: &'static [u8] = Box::leak(menu.into_boxed_slice());
        let (port, _server) = serve_once(menu).await;
        let response = fetch(&format!("gopher://127.0.0.1:{port}/1/big"), AddressSpace::Loopback)
            .await
            .unwrap();
        assert!(response.rest.is_some());
        let page = whole_body(response).await.unwrap();
        let page = std::str::from_utf8(&page).unwrap();
        assert_eq!(page.matches("<a href=").count(), (MAX_RESPONSE_BYTES - 3) / line.len());
        assert!(page.ends_with("</pre></body></html>"));
    }

    /// Reads `response`'s page until it holds `want`, and says how long that took.
    async fn read_until(response: ProtocolResponse, want: &str) -> (String, Duration) {
        let started = std::time::Instant::now();
        let url = Url::parse("gopher://test/").unwrap();
        let observer = Arc::new(gosub_sonar::net::null_emitter::NullEmitter);
        let shared = protocols::stream_body(response.rest.unwrap(), 0, url, started, observer);
        let mut reader =
            crate::net::SharedBody::combined_reader_lossless(PeekBuf::from_vec(response.body.to_vec()), shared);
        let mut page = Vec::new();
        let mut buf = [0u8; 4096];
        while !String::from_utf8_lossy(&page).contains(want) {
            let n = reader.read(&mut buf).await.unwrap();
            assert!(n > 0, "the page ended without {want}");
            page.extend_from_slice(&buf[..n]);
        }
        (String::from_utf8(page).unwrap(), started.elapsed())
    }

    #[tokio::test]
    async fn a_menu_shows_its_first_lines_before_the_rest_arrives() {
        // More than the peek first, so the fetch answers without waiting for the rest.
        let mut first = b"iWelcome\t\t\t\r\n".repeat(80);
        first.extend(b"1Early\t/early\t127.0.0.1\t70\r\n");
        let chunks = vec![first, b"1Late\t/late\t127.0.0.1\t70\r\n.\r\n".to_vec()];
        let port = serve_slowly(chunks, Duration::from_secs(3), Duration::ZERO).await;
        let started = std::time::Instant::now();
        let response = fetch(&format!("gopher://127.0.0.1:{port}/1/"), AddressSpace::Loopback)
            .await
            .unwrap();
        assert_eq!(response.content_type, "text/html; charset=utf-8");
        let (page, _) = read_until(response, ">Early</a>").await;
        assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
        assert!(page.starts_with("<!DOCTYPE html>"));
        assert!(!page.contains("Late"));
    }

    #[tokio::test]
    async fn a_text_file_shows_its_first_lines_before_the_rest_arrives() {
        let mut first = b"a line of text\r\n".repeat(80);
        first.extend(b"..stuffed <early>\r\n");
        let chunks = vec![first, b"late\r\n.\r\n".to_vec()];
        let port = serve_slowly(chunks, Duration::from_secs(3), Duration::ZERO).await;
        let started = std::time::Instant::now();
        let response = fetch(&format!("gopher://127.0.0.1:{port}/0/t.txt"), AddressSpace::Loopback)
            .await
            .unwrap();
        let (page, _) = read_until(response, ".stuffed &lt;early&gt;\n").await;
        assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
        assert!(!page.contains("..stuffed"));
        assert!(!page.contains("late"));
    }

    #[tokio::test]
    async fn a_streamed_page_ends_at_its_terminator_though_the_server_stays() {
        let mut menu = b"iLine\t\t\t\r\n".repeat(200);
        menu.extend(b".\r\n");
        let port = serve_slowly(vec![menu], Duration::ZERO, Duration::from_secs(10)).await;
        let response = fetch(&format!("gopher://127.0.0.1:{port}/1/"), AddressSpace::Loopback)
            .await
            .unwrap();
        let (page, took) = read_until(response, "</html>").await;
        assert!(took < Duration::from_secs(5), "{took:?}");
        assert_eq!(page.matches("Line<br>").count(), 200);
    }

    #[tokio::test]
    async fn a_streamed_latin1_text_file_keeps_its_characters() {
        let mut text = b"Gr\xfc\xdfe aus K\xf6ln\r\n".repeat(100);
        text.extend(b".\r\n");
        let text: &'static [u8] = Box::leak(text.into_boxed_slice());
        assert!(text.len() > socket::PEEK_BYTES);
        let (port, _server) = serve_once(text).await;
        let response = fetch(&format!("gopher://127.0.0.1:{port}/0/k.txt"), AddressSpace::Loopback)
            .await
            .unwrap();
        assert!(response.rest.is_some());
        let page = whole_body(response).await.unwrap();
        let page = std::str::from_utf8(&page).unwrap();
        assert_eq!(page.matches("Gr\u{fc}\u{df}e aus K\u{f6}ln\n").count(), 100);
    }

    #[tokio::test]
    async fn a_long_error_menu_for_a_menu_is_read_whole() {
        let mut menu = b"3'/nope' does not exist\t\terror.host\t1\r\n".to_vec();
        menu.extend(b"1Home\t/\terror.host\t70\r\n".repeat(100));
        let menu: &'static [u8] = Box::leak(menu.into_boxed_slice());
        let (port, _server) = serve_once(menu).await;
        let response = fetch(&format!("gopher://127.0.0.1:{port}/0/nope"), AddressSpace::Loopback)
            .await
            .unwrap();
        assert!(response.rest.is_none());
        let page = std::str::from_utf8(&response.body).unwrap();
        assert!(page.contains("class=\"e\">'/nope' does not exist</pre>"));
    }

    #[tokio::test]
    async fn an_html_item_streams_as_it_is() {
        let mut html = b"<!DOCTYPE html><p>".to_vec();
        html.resize(3 * socket::PEEK_BYTES, b'x');
        let html: &'static [u8] = Box::leak(html.into_boxed_slice());
        let (port, _server) = serve_once(html).await;
        let response = fetch(
            &format!("gopher://127.0.0.1:{port}/h/page.html"),
            AddressSpace::Loopback,
        )
        .await
        .unwrap();
        assert_eq!(response.content_type, "text/html");
        assert_eq!(&whole_body(response).await.unwrap()[..], html);
    }

    #[tokio::test]
    async fn a_binary_over_the_cap_streams_in_full() {
        let big: &'static [u8] = Box::leak(vec![b'x'; MAX_RESPONSE_BYTES + 4096].into_boxed_slice());
        let (port, _server) = serve_once(big).await;
        let response = fetch(&format!("gopher://127.0.0.1:{port}/9/big"), AddressSpace::Loopback)
            .await
            .unwrap();
        assert!(response.rest.is_some(), "a binary this size streams");
        assert_eq!(response.content_type, "application/octet-stream");
        assert_eq!(whole_body(response).await.unwrap().len(), big.len());
    }

    #[tokio::test]
    async fn a_streamed_binary_is_sniffed_by_its_first_bytes() {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.resize(3 * socket::PEEK_BYTES, 0);
        let png: &'static [u8] = Box::leak(png.into_boxed_slice());
        let (port, _server) = serve_once(png).await;
        let response = fetch(&format!("gopher://127.0.0.1:{port}/I/pic"), AddressSpace::Loopback)
            .await
            .unwrap();
        assert_eq!(response.content_type, "image/png");
        assert_eq!(&whole_body(response).await.unwrap()[..], png);
    }

    #[tokio::test]
    async fn a_binary_that_keeps_coming_outlasts_the_response_timeout() {
        let limits = Limits {
            response: Duration::from_millis(100),
            idle: Duration::from_millis(500),
            max_bytes: 64,
        };
        let chunks: Vec<Vec<u8>> = (0..30u8).map(|i| vec![i; 256]).collect();
        let port = serve_slowly(chunks, Duration::from_millis(20), Duration::ZERO).await;
        let started = std::time::Instant::now();
        let response = fetch_under(&format!("gopher://127.0.0.1:{port}/9/slow"), &limits)
            .await
            .unwrap();
        let body = whole_body(response).await.unwrap();
        assert!(started.elapsed() > limits.response);
        assert_eq!(body.len(), 30 * 256);
        assert_eq!(body[29 * 256], 29);
    }

    #[tokio::test]
    async fn a_stalled_binary_times_out() {
        let limits = Limits {
            response: Duration::from_secs(60),
            idle: Duration::from_millis(200),
            max_bytes: MAX_RESPONSE_BYTES,
        };
        // Stalls after the peek: the body ends in the timeout.
        let port = serve_slowly(
            vec![vec![b'x'; 2 * socket::PEEK_BYTES]],
            Duration::ZERO,
            Duration::from_secs(10),
        )
        .await;
        let response = fetch_under(&format!("gopher://127.0.0.1:{port}/9/stall"), &limits)
            .await
            .unwrap();
        let started = std::time::Instant::now();
        let err = whole_body(response).await.unwrap_err();
        assert!(matches!(err, NetError::Timeout(_)), "{err:?}");
        assert!(started.elapsed() < Duration::from_secs(5));

        // Stalls within the peek: the fetch does.
        let port = serve_slowly(vec![b"GIF89a".to_vec()], Duration::ZERO, Duration::from_secs(10)).await;
        let err = fetch_under(&format!("gopher://127.0.0.1:{port}/I/stall"), &limits)
            .await
            .unwrap_err();
        assert!(matches!(err, NetError::Timeout(_)), "{err:?}");
    }

    #[tokio::test]
    async fn an_error_menu_for_a_binary_item_is_an_error_page() {
        // Longer than the peek, so the rest is read after it.
        let mut menu = b"3'/nope' does not exist\t\terror.host\t1\r\n".to_vec();
        menu.extend(b"1Home\t/\terror.host\t70\r\n".repeat(100));
        let menu: &'static [u8] = Box::leak(menu.into_boxed_slice());
        assert!(menu.len() > socket::PEEK_BYTES);
        let (port, _server) = serve_once(menu).await;
        let response = fetch(&format!("gopher://127.0.0.1:{port}/9/nope"), AddressSpace::Loopback)
            .await
            .unwrap();
        assert!(response.rest.is_none());
        let page = std::str::from_utf8(&response.body).unwrap();
        assert!(page.contains("class=\"e\">'/nope' does not exist</pre>"));
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
