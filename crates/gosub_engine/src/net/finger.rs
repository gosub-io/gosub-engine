//! `finger://` (RFC 1288), one of the engine's own [protocols].
//!
//! `finger://<host>:<port>/<query>`: the query is the URL's path without its first `/`,
//! percent-decoded - a user name, `/W <user>` for the long form, or nothing for the list of
//! who is logged in. It goes to the server as one line, CRLF, and the answer is plain text
//! until the server closes, shown as a `<pre>` page.
//!
//! The query goes onto the socket as raw bytes, so control bytes are refused: a line break
//! would make a URL two lines, or a whole other protocol, on any port it can reach. An `@`
//! is refused too: `user@host` asks the server to finger `host` itself (RFC 1288 §2.5.1), a
//! connection made beyond any reach this engine judged. A URL with a query (`?`) is refused
//! rather than half-sent.
//!
//! The connection is [`socket`]'s, with its reach and port rules. Finger's own port, 79, is
//! one the Fetch standard blocks, and is open to finger only.
//!
//! [protocols]: crate::net::protocols

use crate::net::protocols::page::{self, escape, title_for};
use crate::net::protocols::socket::{self, invalid};
use crate::net::protocols::{Protocol, ProtocolError, ProtocolRequest, ProtocolResponse};
use async_trait::async_trait;
use bytes::Bytes;
use percent_encoding::percent_decode_str;
use url::{Host, Url};

/// Finger's port when the URL names none (RFC 1288 §2.1).
pub const DEFAULT_PORT: u16 = 79;

const STYLE: &str = "body{margin:0;padding:16px 24px;font-family:monospace;line-height:1.4}\
pre{margin:0;white-space:pre-wrap}";

/// The finger protocol.
pub struct Finger;

#[async_trait]
impl Protocol for Finger {
    fn scheme(&self) -> &'static str {
        "finger"
    }

    fn setting(&self) -> &'static str {
        "net.finger.enabled"
    }

    fn default_port(&self) -> u16 {
        DEFAULT_PORT
    }

    fn encrypted(&self) -> bool {
        false
    }

    async fn fetch(&self, request: ProtocolRequest<'_>) -> Result<ProtocolResponse, ProtocolError> {
        let (url, reach) = (request.url, request.reach);
        let request = Request::from_url(url).map_err(invalid)?;
        let addrs = socket::resolve(self, url, &request.host, request.port, reach).await?;
        let (body, peer) = socket::exchange(url, &request.host, &addrs, reach, &request.wire_line()).await?;
        Ok(ProtocolResponse {
            content_type: "text/html; charset=utf-8".into(),
            body: Bytes::from(text_page(url, &body)),
            peer_addr: peer,
            certificates: Vec::new(),
            final_url: None,
        })
    }
}

/// One finger request.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Request {
    /// Host to connect to: a name, or an IP address without brackets.
    host: String,
    port: u16,
    query: Vec<u8>,
}

impl Request {
    /// The request `url` stands for, or why it stands for none.
    fn from_url(url: &Url) -> Result<Self, String> {
        if url.scheme() != "finger" {
            return Err(format!("not a finger URL: {url}"));
        }
        let host = match url.host() {
            Some(Host::Domain(d)) if !d.is_empty() => d.to_string(),
            Some(Host::Ipv4(ip)) => ip.to_string(),
            Some(Host::Ipv6(ip)) => ip.to_string(),
            _ => return Err(format!("finger URL without a host: {url}")),
        };
        if !url.username().is_empty() || url.password().is_some() {
            return Err(format!(
                "a finger URL has no user part; the user goes in the path: {url}"
            ));
        }
        if url.query().is_some() {
            return Err(format!("a finger URL has no query part: {url}"));
        }
        let path = url.path().strip_prefix('/').unwrap_or(url.path());
        let query: Vec<u8> = percent_decode_str(path).collect();
        if let Some(&b) = query.iter().find(|&&b| b < 0x20 || b == 0x7f) {
            return Err(format!("finger query with a forbidden byte 0x{b:02x}"));
        }
        if query.contains(&b'@') {
            return Err(format!(
                "finger query {:?} asks the server to finger another host, which is not done",
                String::from_utf8_lossy(&query)
            ));
        }
        Ok(Self {
            host,
            port: url.port().unwrap_or(DEFAULT_PORT),
            query,
        })
    }

    /// The bytes sent to the server: the query, then CRLF.
    fn wire_line(&self) -> Vec<u8> {
        let mut line = self.query.clone();
        line.extend_from_slice(b"\r\n");
        line
    }
}

/// The answer as a page of text.
fn text_page(url: &Url, body: &[u8]) -> String {
    let text = printable(&String::from_utf8_lossy(body));
    page::document(&title_for(url), STYLE, &format!("<pre>{}</pre>", escape(&text)))
}

/// `text` as a terminal would show it, without what a terminal would act on: line ends
/// become `\n`, tabs become spaces to the next stop of 8 columns (finger output lines up
/// columns with them), terminal escape sequences (`ESC [ ... final`) are dropped whole, and
/// every other control character goes too.
fn printable(text: &str) -> String {
    const TAB_STOP: usize = 8;
    let mut out = String::with_capacity(text.len());
    let mut column = 0;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' if chars.peek() == Some(&'\n') => {}
            '\r' | '\n' => {
                out.push('\n');
                column = 0;
            }
            '\t' => {
                let spaces = TAB_STOP - column % TAB_STOP;
                out.extend(std::iter::repeat_n(' ', spaces));
                column += spaces;
            }
            '\u{1b}' => {
                if chars.peek() == Some(&'[') {
                    chars.next();
                    // Parameters and intermediates, up to and including the final byte.
                    for c in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&c) {
                            break;
                        }
                    }
                }
            }
            c if c.is_control() => {}
            c => {
                out.push(c);
                column += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::ssrf::AddressSpace;
    use crate::net::types::NetError;
    use gosub_sonar::net::types::BlockReason;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    fn query(s: &str) -> Result<Vec<u8>, String> {
        Request::from_url(&url(s)).map(|r| r.query)
    }

    /// A one-shot finger server on loopback: answers the first connection with `response`
    /// and hands back the line it read.
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
            let _ = socket.write_all(response).await;
            line
        });
        (port, task)
    }

    async fn fetch(url: &str, reach: AddressSpace) -> Result<ProtocolResponse, NetError> {
        let url = Url::parse(url).unwrap();
        Finger
            .fetch(ProtocolRequest {
                url: &url,
                reach,
                pins: None,
                identity: None,
            })
            .await
            .map_err(|e| e.error)
    }

    #[test]
    fn the_path_is_the_query() {
        assert_eq!(query("finger://h.test/bob").unwrap(), b"bob");
        assert_eq!(query("finger://h.test/").unwrap(), b"");
        assert_eq!(query("finger://h.test").unwrap(), b"");
        assert_eq!(query("finger://h.test/%2FW%20bob").unwrap(), b"/W bob");
        assert_eq!(query("finger://h.test//W%20bob").unwrap(), b"/W bob");
    }

    #[test]
    fn the_port_defaults_to_79() {
        assert_eq!(Request::from_url(&url("finger://h.test/")).unwrap().port, 79);
        assert_eq!(Request::from_url(&url("finger://h.test:7979/")).unwrap().port, 7979);
    }

    #[test]
    fn a_line_break_or_other_control_byte_is_refused() {
        for bad in [
            "finger://h.test/bob%0D%0AQUIT",
            "finger://h.test/bob%0A",
            "finger://h.test/%00",
            "finger://h.test/%1B",
        ] {
            assert!(query(bad).unwrap_err().contains("forbidden byte"), "{bad}");
        }
    }

    #[test]
    fn forwarding_to_another_host_is_refused() {
        assert!(query("finger://h.test/bob@intranet.test").is_err());
        assert!(query("finger://h.test/@intranet.test").is_err());
        assert!(query("finger://h.test/bob%40intranet.test").is_err());
    }

    #[test]
    fn a_query_or_user_part_is_refused_not_dropped() {
        assert!(query("finger://h.test/bob?x").is_err());
        assert!(query("finger://bob@h.test/").is_err());
    }

    #[test]
    fn terminal_escapes_and_controls_are_dropped() {
        assert_eq!(
            printable("\u{1b}[1;31mred\u{1b}[0m\u{7}!\r\nnext\rlast"),
            "red!\nnext\nlast"
        );
        assert_eq!(printable("a\u{1b}c"), "ac");
    }

    #[test]
    fn tabs_line_up_on_stops_of_8() {
        assert_eq!(printable("bob\tName\n\tx"), "bob     Name\n        x");
        assert_eq!(printable("12345678\tx"), "12345678        x");
        assert_eq!(printable("\u{1b}[1mab\u{1b}[0m\tx"), "ab      x");
    }

    #[tokio::test]
    async fn an_answer_is_fetched_and_shown_as_text() {
        let (port, server) = serve_once(b"Login: bob\r\nPlan:\r\n<script>alert(1)</script>\r\n").await;
        let response = fetch(&format!("finger://127.0.0.1:{port}/bob"), AddressSpace::Loopback)
            .await
            .unwrap();
        assert_eq!(server.await.unwrap(), b"bob\r\n");
        assert_eq!(response.content_type, "text/html; charset=utf-8");
        assert_eq!(response.peer_addr.map(|a| a.port()), Some(port));
        let page = std::str::from_utf8(&response.body).unwrap();
        assert!(page.contains(&format!("<title>127.0.0.1:{port}/bob</title>")), "{page}");
        assert!(
            page.contains("<pre>Login: bob\nPlan:\n&lt;script&gt;alert(1)&lt;/script&gt;\n</pre>"),
            "{page}"
        );
    }

    #[tokio::test]
    async fn an_empty_query_lists_users() {
        let (port, server) = serve_once(b"nobody here\r\n").await;
        fetch(&format!("finger://127.0.0.1:{port}/"), AddressSpace::Loopback)
            .await
            .unwrap();
        assert_eq!(server.await.unwrap(), b"\r\n");
    }

    #[tokio::test]
    async fn loopback_is_refused_to_a_public_reach() {
        let (port, _server) = serve_once(b"").await;
        let err = fetch(&format!("finger://127.0.0.1:{port}/bob"), AddressSpace::Public)
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
    async fn another_service_s_port_is_refused_but_finger_s_own_is_not() {
        let err = fetch("finger://127.0.0.1:25/bob", AddressSpace::Loopback)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            NetError::Blocked {
                reason: BlockReason::BadPort,
                ..
            }
        ));
        // Nothing (or something) listens on 79 here; either way the port is not refused.
        if let Err(err) = fetch("finger://127.0.0.1/bob", AddressSpace::Loopback).await {
            assert!(!matches!(err, NetError::Blocked { .. }), "{err:?}");
        }
    }
}
