//! `gemini://`, one of the engine's own [protocols].
//!
//! A request is the URL, CRLF, sent over TLS to port 1965. The response is a header line, a
//! two-digit status and a meta field, and then, for a success, the body until the server
//! closes:
//! - `1x` asks for input, which the engine cannot send yet: the prompt is shown as a page,
//! - `2x` is the body, under the media type in the meta: gemtext ([`gemtext`]) becomes a page,
//!   other text a `<pre>` page, HTML is served as it is and anything else raw, so images
//!   display and the rest goes to the download offer,
//! - `3x` is a redirect, shown as a page linking to where it points rather than followed,
//! - `4x` and `5x` are failures, and `6x` asks for a client certificate the engine does not
//!   have: each is a page with the server's message.
//!
//! Servers mostly sign their own certificates, so the connection trusts a certificate on
//! first use ([`tls`]): the first visit to a host and port reports the certificate, which the
//! broker pins, and a later visit showing another is refused before the request is sent.
//!
//! Text in a charset other than UTF-8 is read as UTF-8 all the same.
//!
//! The connection is [`socket`]'s, with its reach and port rules.
//!
//! [protocols]: crate::net::protocols

mod gemtext;
#[cfg(test)]
pub(crate) mod testing;
mod tls;

use crate::net::protocols::page::{self, escape, title_for};
use crate::net::protocols::socket::{self, invalid, Close};
use crate::net::protocols::{Protocol, ProtocolRequest, ProtocolResponse};
use crate::net::types::NetError;
use async_trait::async_trait;
use bytes::Bytes;
use cow_utils::CowUtils;
use url::{Host, Url};

/// Gemini's port when the URL names none.
pub const DEFAULT_PORT: u16 = 1965;

/// Longest URL a request may carry, in bytes.
const MAX_URL_BYTES: usize = 1024;

/// Longest meta field a header may carry, in bytes.
const MAX_META_BYTES: usize = 1024;

/// The gemini protocol.
pub struct Gemini;

#[async_trait]
impl Protocol for Gemini {
    fn scheme(&self) -> &'static str {
        "gemini"
    }

    fn setting(&self) -> &'static str {
        "net.gemini.enabled"
    }

    fn default_port(&self) -> u16 {
        DEFAULT_PORT
    }

    fn encrypted(&self) -> bool {
        true
    }

    fn pins_certificates(&self) -> bool {
        true
    }

    async fn fetch(&self, request: ProtocolRequest<'_>) -> Result<ProtocolResponse, NetError> {
        let (url, reach) = (request.url, request.reach);
        let gemini = Request::from_url(url).map_err(invalid)?;
        let addrs = socket::resolve(self, url, &gemini.host, gemini.port, reach).await?;
        let tcp = socket::connect(url, &gemini.host, &addrs, reach).await?;
        let peer = tcp.peer_addr().ok();
        let (mut stream, certificate) = tls::handshake(&gemini.host, gemini.port, tcp, request.pin).await?;
        let response = socket::talk(&mut stream, &gemini.host, &gemini.line, Close::Unclean).await?;
        let (content_type, body) =
            present(url, &response).map_err(|e| invalid(format!("{} answered {url}: {e}", gemini.host)))?;
        Ok(ProtocolResponse {
            content_type,
            body: Bytes::from(body),
            peer_addr: peer,
            certificate: Some(certificate),
        })
    }
}

/// One gemini request.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Request {
    /// Host to connect to: a name, or an IP address without brackets.
    host: String,
    port: u16,
    /// The bytes sent to the server: the URL without its fragment, then CRLF.
    line: Vec<u8>,
}

impl Request {
    /// The request `url` stands for, or why it stands for none.
    fn from_url(url: &Url) -> Result<Self, String> {
        if url.scheme() != "gemini" {
            return Err(format!("not a gemini URL: {url}"));
        }
        let host = match url.host() {
            Some(Host::Domain(d)) if !d.is_empty() => d.to_string(),
            Some(Host::Ipv4(ip)) => ip.to_string(),
            Some(Host::Ipv6(ip)) => ip.to_string(),
            _ => return Err(format!("gemini URL without a host: {url}")),
        };
        if !url.username().is_empty() || url.password().is_some() {
            return Err(format!("a gemini URL has no user part: {url}"));
        }
        let mut target = url.clone();
        target.set_fragment(None);
        // An empty path and `/` are the same page; servers answer the first with a redirect.
        if target.path().is_empty() {
            target.set_path("/");
        }
        let target = target.as_str();
        if target.len() > MAX_URL_BYTES {
            return Err(format!("gemini URL longer than {MAX_URL_BYTES} bytes"));
        }
        // The URL parser escapes these already; the line must stay one line regardless.
        if target.bytes().any(|b| b < 0x20 || b == 0x7f) {
            return Err(format!("gemini URL with a control byte: {url}"));
        }
        Ok(Self {
            host,
            port: url.port().unwrap_or(DEFAULT_PORT),
            line: format!("{target}\r\n").into_bytes(),
        })
    }
}

/// A response's header: the status, its meta field, and the body after them.
fn header(response: &[u8]) -> Result<(u8, &str, &[u8]), String> {
    // Status, space, meta, CRLF.
    let longest = (2 + 1 + MAX_META_BYTES + 2).min(response.len());
    let end = response[..longest]
        .iter()
        .position(|&b| b == b'\n')
        .ok_or("no header line")?;
    let line = &response[..end];
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    let line = std::str::from_utf8(line).map_err(|_| "a header that is not UTF-8")?;
    let (code, meta) = line.split_at_checked(2).ok_or("a header without a status")?;
    let status: u8 = match code.as_bytes() {
        [tens @ b'1'..=b'6', units @ b'0'..=b'9'] => (tens - b'0') * 10 + (units - b'0'),
        _ => return Err(format!("status {code:?}, which is none gemini has")),
    };
    let meta = match meta {
        "" => "",
        meta => meta.strip_prefix(' ').ok_or("a status run on into its meta field")?,
    };
    Ok((status, meta, &response[end + 1..]))
}

/// What a response is shown as: its media type and body.
fn present(url: &Url, response: &[u8]) -> Result<(String, Vec<u8>), String> {
    let (status, meta, body) = header(response)?;
    Ok(match status / 10 {
        2 => content(url, meta, body),
        1 => html(status_page(
            url,
            status,
            &format!(
                "<p>{}</p><p class=\"u\">The server asks for input, which this browser cannot send yet.</p>",
                escape(meta)
            ),
        )),
        3 => html(status_page(url, status, &redirect(url, meta))),
        6 => html(status_page(
            url,
            status,
            &format!(
                "<p>{}</p><p class=\"u\">The server asks for a client certificate, which this browser \
cannot present yet.</p>",
                escape(meta)
            ),
        )),
        _ => html(status_page(url, status, &format!("<p>{}</p>", escape(meta)))),
    })
}

/// A success: the body under the media type in `meta`, which is gemtext in UTF-8 when empty.
fn content(url: &Url, meta: &str, body: &[u8]) -> (String, Vec<u8>) {
    let mut parts = meta.split(';');
    let essence = parts.next().unwrap_or_default().trim().cow_to_ascii_lowercase();
    let lang = parts.find_map(|param| {
        let (name, value) = param.split_once('=')?;
        name.trim()
            .eq_ignore_ascii_case("lang")
            .then(|| value.trim().trim_matches('"'))
    });
    match &*essence {
        "" | "text/gemini" => html(gemtext::page(url, &String::from_utf8_lossy(body), lang)),
        "text/html" => ("text/html; charset=utf-8".into(), body.to_vec()),
        text if text.starts_with("text/") => html(page::document(
            &title_for(url),
            gemtext::STYLE,
            &format!("<pre>{}</pre>", escape(&String::from_utf8_lossy(body))),
        )),
        other if is_media_type(other) => (other.to_string(), body.to_vec()),
        _ => ("application/octet-stream".into(), body.to_vec()),
    }
}

/// Whether `s` is a `type/subtype` of token characters, which the rest of the pipeline may
/// route on.
fn is_media_type(s: &str) -> bool {
    let token = |t: &str| {
        !t.is_empty()
            && t.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$&-^_.+".contains(&b))
    };
    s.split_once('/').is_some_and(|(t, sub)| token(t) && token(sub))
}

/// A redirect's body: where it points, linked when a gemini page may go there.
fn redirect(url: &Url, meta: &str) -> String {
    match url.join(meta) {
        Ok(to) if matches!(to.scheme(), "gemini" | "http" | "https") => format!(
            "<p>The page is at <a href=\"{}\">{}</a>.</p>",
            escape(to.as_str()),
            escape(to.as_str())
        ),
        _ => format!(
            "<p>The page is at {}, which this browser does not go to.</p>",
            escape(meta)
        ),
    }
}

/// A page for a status that has no body, headed by what the status means.
fn status_page(url: &Url, status: u8, body: &str) -> String {
    page::document(
        &title_for(url),
        gemtext::STYLE,
        &format!("<h1>{} ({status})</h1>{body}", escape(status_name(status))),
    )
}

fn status_name(status: u8) -> &'static str {
    match status {
        11 => "Sensitive input requested",
        30 => "Temporary redirect",
        31 => "Permanent redirect",
        41 => "Server unavailable",
        42 => "CGI error",
        43 => "Proxy error",
        44 => "Slow down",
        51 => "Not found",
        52 => "Gone",
        53 => "Proxy request refused",
        59 => "Bad request",
        61 => "Certificate not authorised",
        62 => "Certificate not valid",
        _ => match status / 10 {
            1 => "Input requested",
            3 => "Redirect",
            4 => "Temporary failure",
            5 => "Permanent failure",
            _ => "Client certificate required",
        },
    }
}

fn html(page: String) -> (String, Vec<u8>) {
    ("text/html; charset=utf-8".into(), page.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::protocols::known_hosts::CertPin;
    use crate::net::ssrf::AddressSpace;
    use gosub_sonar::net::tls::TlsErrorKind;
    use gosub_sonar::net::types::BlockReason;
    use testing::{serve, A, A_CERT, B, B_CERT, FORGED};

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    fn line(s: &str) -> Result<String, String> {
        Request::from_url(&url(s)).map(|r| String::from_utf8(r.line).unwrap())
    }

    async fn fetch(url: &str, pin: Option<&CertPin>) -> Result<ProtocolResponse, NetError> {
        let url = Url::parse(url).unwrap();
        Gemini
            .fetch(ProtocolRequest {
                url: &url,
                reach: AddressSpace::Loopback,
                pin,
            })
            .await
    }

    fn body(response: &ProtocolResponse) -> &str {
        std::str::from_utf8(&response.body).unwrap()
    }

    #[test]
    fn the_request_is_the_url_without_its_fragment() {
        assert_eq!(line("gemini://h.test/a?b#c").unwrap(), "gemini://h.test/a?b\r\n");
        assert_eq!(line("gemini://h.test").unwrap(), "gemini://h.test/\r\n");
        assert_eq!(line("gemini://h.test:1966/").unwrap(), "gemini://h.test:1966/\r\n");
        assert_eq!(Request::from_url(&url("gemini://h.test/")).unwrap().port, 1965);
    }

    #[test]
    fn a_user_part_or_an_overlong_url_is_refused() {
        assert!(line("gemini://bob@h.test/").is_err());
        assert!(line(&format!("gemini://h.test/{}", "a".repeat(1024))).is_err());
        assert!(line(&format!(
            "gemini://h.test/{}",
            "a".repeat(1024 - "gemini://h.test/".len())
        ))
        .is_ok());
    }

    #[test]
    fn a_header_is_a_status_and_a_meta_field() {
        assert_eq!(
            header(b"20 text/gemini\r\nbody").unwrap(),
            (20, "text/gemini", &b"body"[..])
        );
        assert_eq!(header(b"20\r\n").unwrap(), (20, "", &b""[..]));
        assert_eq!(header(b"51 gone\n").unwrap(), (51, "gone", &b""[..]));
        for bad in [
            &b"2 x\r\n"[..],
            b"70 x\r\n",
            b"20x\r\n",
            b"ab\r\n",
            b"20 no line end",
            b"",
        ] {
            assert!(header(bad).is_err(), "{:?}", String::from_utf8_lossy(bad));
        }
        let mut long = b"20 ".to_vec();
        long.extend(std::iter::repeat_n(b'x', MAX_META_BYTES + 1));
        long.extend(b"\r\n");
        assert!(header(&long).is_err());
    }

    #[test]
    fn a_success_is_shown_by_its_media_type() {
        let at = url("gemini://h.test/");
        let shown = |r: &[u8]| present(&at, r).unwrap();
        let (ty, page) = shown(b"20 text/gemini; lang=nl\r\n# Hoi");
        assert_eq!(ty, "text/html; charset=utf-8");
        let page = String::from_utf8(page).unwrap();
        assert!(
            page.contains("<html lang=\"nl\">") && page.contains("<h1>Hoi</h1>"),
            "{page}"
        );
        assert!(String::from_utf8(shown(b"20\r\n# x").1).unwrap().contains("<h1>x</h1>"));
        let (ty, page) = shown(b"20 text/plain\r\n# <x>");
        assert_eq!(ty, "text/html; charset=utf-8");
        assert!(String::from_utf8(page).unwrap().contains("<pre># &lt;x&gt;</pre>"));
        assert_eq!(
            shown(b"20 text/html\r\n<b>"),
            ("text/html; charset=utf-8".into(), b"<b>".to_vec())
        );
        assert_eq!(
            shown(b"20 IMAGE/PNG\r\n\x89PNG"),
            ("image/png".into(), b"\x89PNG".to_vec())
        );
        assert_eq!(shown(b"20 bogus\r\nx").0, "application/octet-stream");
        assert_eq!(shown(b"20 a/b\"<\r\nx").0, "application/octet-stream");
    }

    #[test]
    fn other_statuses_are_pages_with_the_server_s_message() {
        let at = url("gemini://h.test/dir/");
        let shown = |r: &[u8]| String::from_utf8(present(&at, r).unwrap().1).unwrap();
        let page = shown(b"10 Your name?\r\n");
        assert!(
            page.contains("<h1>Input requested (10)</h1><p>Your name?</p>"),
            "{page}"
        );
        assert!(shown(b"51 <nope>\r\n").contains("<h1>Not found (51)</h1><p>&lt;nope&gt;</p>"));
        assert!(shown(b"42 x\r\n").contains("<h1>CGI error (42)</h1>"));
        assert!(shown(b"48 x\r\n").contains("<h1>Temporary failure (48)</h1>"));
        assert!(shown(b"60 who?\r\n").contains("client certificate"));
        assert!(shown(b"31 ../new\r\n").contains("<a href=\"gemini://h.test/new\">gemini://h.test/new</a>"));
        assert!(shown(b"30 file:///etc/passwd\r\n").contains("file:///etc/passwd, which this browser does not go to"));
    }

    #[tokio::test]
    async fn a_first_visit_reports_the_certificate_and_a_pinned_one_passes() {
        let (port, server) = serve(&[A], b"20 text/gemini\r\n# Hello\r\n", true).await;
        let first = fetch(&format!("gemini://127.0.0.1:{port}/x#frag"), None).await.unwrap();
        assert_eq!(
            server.await.unwrap()[0].as_deref().unwrap(),
            format!("gemini://127.0.0.1:{port}/x\r\n").as_bytes()
        );
        assert_eq!(first.certificate, Some(CertPin::of(A_CERT)));
        assert_eq!(first.peer_addr.map(|a| a.port()), Some(port));
        assert!(body(&first).contains("<h1>Hello</h1>"), "{}", body(&first));

        let (port, _server) = serve(&[A], b"20 text/gemini\r\nagain", true).await;
        let pin = CertPin::of(A_CERT);
        let again = fetch(&format!("gemini://127.0.0.1:{port}/"), Some(&pin)).await.unwrap();
        assert!(body(&again).contains("<p>again</p>"));
    }

    #[tokio::test]
    async fn another_certificate_than_the_pinned_one_is_refused_before_the_request() {
        let (port, server) = serve(&[B], b"20 text/gemini\r\nsecret", true).await;
        let pin = CertPin::of(A_CERT);
        let err = fetch(&format!("gemini://127.0.0.1:{port}/"), Some(&pin))
            .await
            .unwrap_err();
        let NetError::Tls(tls) = err else {
            panic!("not a TLS error: {err:?}")
        };
        assert_eq!(tls.kind, TlsErrorKind::InvalidCertificate);
        assert!(tls.message.contains(pin.as_str()), "{}", tls.message);
        assert!(tls.message.contains(CertPin::of(B_CERT).as_str()), "{}", tls.message);
        assert!(tls.fingerprint.is_none());
        assert_eq!(server.await.unwrap(), [None], "the request was sent");
    }

    #[tokio::test]
    async fn a_certificate_shown_without_its_key_is_refused_on_a_first_visit_too() {
        let (port, server) = serve(&[FORGED], b"20 text/gemini\r\nforged", true).await;
        let err = fetch(&format!("gemini://127.0.0.1:{port}/"), None).await.unwrap_err();
        assert!(matches!(err, NetError::Tls(_)), "{err:?}");
        assert_eq!(server.await.unwrap(), [None], "the request was sent");
    }

    #[tokio::test]
    async fn a_server_closing_without_close_notify_still_answers() {
        let (port, _server) = serve(&[A], b"20 text/plain\r\nbye", false).await;
        let response = fetch(&format!("gemini://127.0.0.1:{port}/"), None).await.unwrap();
        assert!(body(&response).contains("<pre>bye</pre>"), "{}", body(&response));
    }

    #[tokio::test]
    async fn loopback_is_refused_to_a_public_reach() {
        let url = Url::parse("gemini://127.0.0.1:1966/").unwrap();
        let err = Gemini
            .fetch(ProtocolRequest {
                url: &url,
                reach: AddressSpace::Public,
                pin: None,
            })
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
}
