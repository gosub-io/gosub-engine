//! `gemini://`, one of the engine's own [protocols].
//!
//! A request is the URL, CRLF, sent over TLS to port 1965. The response is a header line, a
//! two-digit status and a meta field, and then, for a success, the body until the server
//! closes:
//! - `1x` asks for input: a page with the prompt and a form, `11` with a masked field.
//!   Submitting it requests the same URL with the input as the whole query
//!   ([`Protocol::form_query`]),
//! - `2x` is the body, under the media type in the meta: gemtext ([`gemtext`]) becomes a page,
//!   other text a `<pre>` page, HTML is served as it is and anything else raw, so images
//!   display and the rest goes to the download offer,
//! - `3x` is a redirect. One to a gemini URL is followed, up to [`MAX_REDIRECTS`] times in
//!   all, and the page commits under where it ended. One to another scheme is shown as a
//!   page linking there,
//! - `4x` and `5x` are failures: each is a page with the server's message. After a `44`,
//!   slow down, the server is left alone for a while that doubles each time it says so again
//!   ([`slow_down`]),
//! - `6x` asks for a client certificate, or another one: the navigation fails with what the
//!   server said (`IdentityAsked`), for the embedder to ask the user. A request presents
//!   the zone's identity for its URL on every hop inside that identity's scope
//!   (`net::protocols::identities`).
//!
//! Servers mostly sign their own certificates, so the connection trusts a certificate on
//! first use ([`tls`]): the first visit to a host and port reports the certificate, which the
//! broker pins, and a later visit showing another is refused before the request is sent.
//! Each server a request reaches is held to the zone's pin for it
//! ([`ProtocolRequest::pins`]), or on a first visit to the certificate it showed first in
//! that request. A changed certificate is offered for trusting only at the server the page
//! was asked for: one a redirect led to is decided about by visiting it.
//!
//! Text comes in the charset its media type names, UTF-8 when it names none, and is decoded
//! before it is shown.
//!
//! The connection is [`socket`]'s, with its reach and port rules.
//!
//! [protocols]: crate::net::protocols

mod gemtext;
mod slow_down;
#[cfg(test)]
pub(crate) mod testing;
mod tls;

use crate::net::protocols::identities::{ClientIdentity, IdentityAsked};
use crate::net::protocols::known_hosts::{key, CertPin, ServerPin};
use crate::net::protocols::page::{self, escape, title_for};
use crate::net::protocols::socket::{self, invalid, Close};
use crate::net::protocols::{Protocol, ProtocolError, ProtocolRequest, ProtocolResponse};
use crate::net::types::NetError;
use async_trait::async_trait;
use bytes::Bytes;
use cow_utils::CowUtils;
use std::collections::HashMap;
use url::{Host, Url};

/// Gemini's port when the URL names none.
pub const DEFAULT_PORT: u16 = 1965;

/// Longest URL a request may carry, in bytes.
const MAX_URL_BYTES: usize = 1024;

/// Longest meta field a header may carry, in bytes.
const MAX_META_BYTES: usize = 1024;

/// Redirects followed for one request before giving up, as the specification advises.
pub const MAX_REDIRECTS: usize = 5;

/// What a query's input is percent-encoded against: all but the unreserved characters, so a
/// space is `%20` and never `+`.
const INPUT: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

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

    /// A form's input is the whole query, percent-encoded, when the form has one field.
    fn form_query(&self, data: &[(String, String)]) -> Option<String> {
        match data {
            [(_, input)] => Some(percent_encoding::utf8_percent_encode(input, INPUT).to_string()),
            _ => None,
        }
    }

    async fn fetch(&self, request: ProtocolRequest<'_>) -> Result<ProtocolResponse, ProtocolError> {
        // What first visits showed goes back on a failure too: a server that answered is
        // pinned when a later answer, or a later hop, fails the request.
        let mut seen = Vec::new();
        let followed = self.follow(request, &mut seen).await;
        followed.map_err(|error| ProtocolError {
            error,
            certificates: seen,
        })
    }
}

impl Gemini {
    /// [`Protocol::fetch`] proper: follows the redirects, holding each server to its pin, and
    /// adds the certificate each first visit showed to `seen`.
    async fn follow(
        &self,
        request: ProtocolRequest<'_>,
        seen: &mut Vec<ServerPin>,
    ) -> Result<ProtocolResponse, NetError> {
        let mut url = request.url.clone();
        let asked_for = key(&url, DEFAULT_PORT);
        // The certificate each server reached so far is held to: the zone's pin (`true`), or
        // what it showed first in this request on a first visit.
        let mut held: HashMap<(String, u16), (CertPin, bool)> = HashMap::new();
        let mut hops = 0;
        loop {
            let server = key(&url, DEFAULT_PORT).ok_or_else(|| invalid(format!("gemini URL without a host: {url}")))?;
            if let Some(wait) = slow_down::waiting(&server) {
                if wait > slow_down::LONGEST_WAIT {
                    return Ok(ProtocolResponse {
                        content_type: "text/html; charset=utf-8".into(),
                        body: Bytes::from(held_back(&url, wait)),
                        peer_addr: None,
                        certificates: std::mem::take(seen),
                        final_url: (hops > 0).then_some(url),
                    });
                }
                tokio::time::sleep(wait).await;
            }
            let pin = match (held.get(&server), request.pins) {
                (Some(held), _) => Some(held.clone()),
                (None, Some(pins)) => pins
                    .pin(&server.0, server.1)
                    .await
                    .map_err(|e| invalid(format!("no pin to hold {url} to: {e}")))?
                    .map(|pin| (pin, true)),
                (None, None) => None,
            };
            let zone_pin = pin.as_ref().is_some_and(|(_, zone)| *zone);
            // Never above the path the identity was made for, nor on another server.
            let identity = request
                .identity
                .filter(|identity| identity.scope.covers(&url, DEFAULT_PORT));
            let exchanged = self
                .exchange(&url, request.reach, pin.as_ref().map(|(pin, _)| pin), identity)
                .await;
            let (response, peer, certificate) = match exchanged {
                // Only the zone's pin for the server the page was asked for is the user's to
                // replace: a first visit has no pin yet, and a server a redirect led to is
                // decided about by visiting it.
                Err(NetError::Tls(mut tls))
                    if tls.fingerprint.is_some() && !(zone_pin && Some(&server) == asked_for.as_ref()) =>
                {
                    tls.fingerprint = None;
                    tls.message = if zone_pin {
                        format!(
                            "{url}, which {} redirected to, showed another certificate than the one \
pinned for it; visit it to decide about it",
                            request.url
                        )
                    } else {
                        format!("{url} showed another certificate than earlier in this request's redirects, on a first visit")
                    };
                    return Err(NetError::Tls(tls));
                }
                exchanged => exchanged?,
            };
            if !held.contains_key(&server) {
                if pin.is_none() {
                    seen.push(ServerPin {
                        host: server.0.clone(),
                        port: server.1,
                        pin: certificate.clone(),
                    });
                }
                held.insert(server.clone(), pin.unwrap_or((certificate, false)));
            }
            let status = header(&response).map(|(status, ..)| status).ok();
            let delay = slow_down::answered(&server, status == Some(44));
            if let Ok((status @ 60..=69, meta, _)) = header(&response) {
                return Err(IdentityAsked {
                    url,
                    status,
                    message: meta.to_string(),
                    presented: identity.is_some(),
                }
                .into_net_error());
            }
            let target = followed(&url, &response).map_err(|e| invalid(format!("{url} answered: {e}")))?;
            let target = match target {
                Some(mut target) if hops < MAX_REDIRECTS => {
                    hops += 1;
                    // A target without a fragment keeps the one the page was asked with, as
                    // a redirect on the web does.
                    if target.fragment().is_none() {
                        target.set_fragment(url.fragment());
                    }
                    url = target;
                    continue;
                }
                target => target,
            };
            let (content_type, body) = match target {
                Some(target) => html(status_page(
                    &url,
                    30,
                    &format!(
                        "<p>The page was redirected {MAX_REDIRECTS} times and still points on, to \
<a href=\"{}\">{}</a>.</p>",
                        escape(target.as_str()),
                        escape(target.as_str())
                    ),
                )),
                None => present(&url, &response, delay).map_err(|e| invalid(format!("{url} answered: {e}")))?,
            };
            return Ok(ProtocolResponse {
                content_type,
                body: Bytes::from(body),
                peer_addr: peer,
                certificates: std::mem::take(seen),
                final_url: (hops > 0).then_some(url),
            });
        }
    }

    /// One request and its whole response: connect to `url`'s server, hold its certificate
    /// to `pin`, present `identity` if it asks for one, send the request line and read until
    /// the server closes. Returns the response, the peer and the certificate the server
    /// showed.
    async fn exchange(
        &self,
        url: &Url,
        reach: crate::net::ssrf::AddressSpace,
        pin: Option<&CertPin>,
        identity: Option<&ClientIdentity>,
    ) -> Result<(Vec<u8>, Option<std::net::SocketAddr>, CertPin), NetError> {
        let gemini = Request::from_url(url).map_err(invalid)?;
        let addrs = socket::resolve(self, url, &gemini.host, gemini.port, reach).await?;
        let tcp = socket::connect(url, &gemini.host, &addrs, reach).await?;
        let peer = tcp.peer_addr().ok();
        let (mut stream, certificate) = tls::handshake(&gemini.host, gemini.port, tcp, pin, identity).await?;
        let response = socket::talk(&mut stream, &gemini.host, &gemini.line, Close::Unclean).await?;
        Ok((response, peer, certificate))
    }
}

/// Where a response sends the request on to: a redirect's target, when it is a gemini URL.
/// `None` for any other response, which is shown, for a redirect to another scheme, which is
/// shown as a link, and for a redirect to nowhere: an empty target is no URI the
/// specification lets a redirect name.
fn followed(url: &Url, response: &[u8]) -> Result<Option<Url>, String> {
    let (status, meta, _) = header(response)?;
    if status / 10 != 3 || meta.is_empty() {
        return Ok(None);
    }
    Ok(url.join(meta).ok().filter(|to| to.scheme() == "gemini"))
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

/// What a response is shown as: its media type and body. `delay` is how long the server is
/// now left alone, when it said slow down.
fn present(url: &Url, response: &[u8], delay: Option<std::time::Duration>) -> Result<(String, Vec<u8>), String> {
    let (status, meta, body) = header(response)?;
    if let (44, Some(delay)) = (status, delay) {
        return Ok(html(status_page(
            url,
            status,
            &format!(
                "<p>{}</p><p class=\"u\">This browser asks the server again in {} at the earliest: \
<a href=\"{}\">try again</a>.</p>",
                escape(meta),
                slow_down::in_words(delay),
                escape(url.as_str())
            ),
        )));
    }
    Ok(match status / 10 {
        2 => content(url, meta, body),
        1 => html(status_page(url, status, &input_form(url, status, meta))),
        3 => html(status_page(url, status, &redirect(url, meta))),
        _ => html(status_page(url, status, &format!("<p>{}</p>", escape(meta)))),
    })
}

/// The page a request gets that was not sent, because its server said slow down and is left
/// alone for another `wait`.
fn held_back(url: &Url, wait: std::time::Duration) -> String {
    status_page(
        url,
        44,
        &format!(
            "<p>The server asked this browser to slow down, so the request was not sent. It is \
sent again in {} at the earliest: <a href=\"{}\">try again</a>.</p>",
            slow_down::in_words(wait),
            escape(url.as_str())
        ),
    )
}

/// A form asking for what the prompt in `meta` asks, submitted to `url` without its query:
/// the input replaces it. Sensitive input (`11`) is typed into a masked field.
fn input_form(url: &Url, status: u8, meta: &str) -> String {
    let mut action = url.clone();
    action.set_query(None);
    action.set_fragment(None);
    let kind = if status == 11 { "password" } else { "text" };
    let prompt = if meta.is_empty() {
        "The server asks for input."
    } else {
        meta
    };
    format!(
        "<form method=\"get\" action=\"{}\"><p><label for=\"input\">{}</label></p>\
<p><input id=\"input\" name=\"input\" type=\"{kind}\" autofocus> <button type=\"submit\">Send</button></p></form>",
        escape(action.as_str()),
        escape(prompt)
    )
}

/// A success: the body under the media type in `meta`, which is gemtext in UTF-8 when empty.
/// Text is decoded from its `charset` first, so every page goes on in UTF-8.
fn content(url: &Url, meta: &str, body: &[u8]) -> (String, Vec<u8>) {
    let mut parts = meta.split(';');
    let essence = parts.next().unwrap_or_default().trim().cow_to_ascii_lowercase();
    let (mut lang, mut charset) = (None, None);
    for param in parts {
        let Some((name, value)) = param.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"');
        match &*name.trim().cow_to_ascii_lowercase() {
            "lang" => lang = lang.or(Some(value)),
            "charset" => charset = charset.or(Some(value)),
            _ => {}
        }
    }
    match &*essence {
        "" | "text/gemini" => html(gemtext::page(url, &text(body, charset), lang)),
        "text/html" => html(text(body, charset).into_owned()),
        text_type if text_type.starts_with("text/") => html(page::document(
            &title_for(url),
            gemtext::STYLE,
            &format!("<pre>{}</pre>", escape(&text(body, charset))),
        )),
        other if is_media_type(other) => (other.to_string(), body.to_vec()),
        _ => ("application/octet-stream".into(), body.to_vec()),
    }
}

/// A text body decoded from `charset`, the encoding the WHATWG Encoding standard gives the
/// label: UTF-8 when there is none, as the specification has a client assume, and when the
/// label names no encoding. A byte order mark for that encoding is not part of the text.
fn text<'a>(body: &'a [u8], charset: Option<&str>) -> std::borrow::Cow<'a, str> {
    let encoding = charset
        .and_then(|label| encoding_rs::Encoding::for_label(label.as_bytes()))
        .unwrap_or(encoding_rs::UTF_8);
    encoding.decode_with_bom_removal(body).0
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
    if meta.is_empty() {
        return "<p>The server redirected the page without saying where to.</p>".into();
    }
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
        // A success is the page itself and a `6x` a failed navigation, so neither has one.
        _ => match status / 10 {
            1 => "Input requested",
            3 => "Redirect",
            4 => "Temporary failure",
            _ => "Permanent failure",
        },
    }
}

fn html(page: String) -> (String, Vec<u8>) {
    ("text/html; charset=utf-8".into(), page.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::protocols::identities::IdentityScope;
    use crate::net::protocols::known_hosts::{CertPin, Pins};
    use crate::net::ssrf::AddressSpace;
    use gosub_sonar::net::tls::TlsErrorKind;
    use gosub_sonar::net::types::BlockReason;
    use testing::{serve, serve_each, serve_tls, Tls, A, A_CERT, B, B_CERT, FORGED};

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    fn line(s: &str) -> Result<String, String> {
        Request::from_url(&url(s)).map(|r| String::from_utf8(r.line).unwrap())
    }

    /// The zone's pins in a test: these servers', and none for the rest.
    struct ZonePins(Vec<ServerPin>);

    #[async_trait]
    impl Pins for ZonePins {
        async fn pin(&self, host: &str, port: u16) -> Result<Option<CertPin>, String> {
            Ok(self
                .0
                .iter()
                .find(|s| s.host == host && s.port == port)
                .map(|s| s.pin.clone()))
        }
    }

    fn pinned(host: &str, port: u16, cert: &[u8]) -> ServerPin {
        ServerPin {
            host: host.into(),
            port,
            pin: CertPin::of(cert),
        }
    }

    /// Fetch `url` with `pin` as the zone's pin for its server, if any.
    async fn fetch(url: &str, pin: Option<&CertPin>) -> Result<ProtocolResponse, NetError> {
        let url = Url::parse(url).unwrap();
        let (host, port) = key(&url, DEFAULT_PORT).unwrap();
        let pins = pin.map(|pin| ServerPin {
            host,
            port,
            pin: pin.clone(),
        });
        fetch_with(&url, &ZonePins(pins.into_iter().collect())).await
    }

    async fn fetch_with(url: &Url, pins: &ZonePins) -> Result<ProtocolResponse, NetError> {
        Gemini
            .fetch(ProtocolRequest {
                url,
                reach: AddressSpace::Loopback,
                pins: Some(pins),
                identity: None,
            })
            .await
            .map_err(|e| e.error)
    }

    /// The failure of a fetch as [`fetch_with`] makes it, with what it reports besides its
    /// error.
    async fn failure(url: &Url, pins: &ZonePins) -> ProtocolError {
        Gemini
            .fetch(ProtocolRequest {
                url,
                reach: AddressSpace::Loopback,
                pins: Some(pins),
                identity: None,
            })
            .await
            .expect_err("the fetch failed")
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
        let shown = |r: &[u8]| present(&at, r, None).unwrap();
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
    fn text_is_decoded_from_its_charset() {
        let at = url("gemini://h.test/");
        let shown = |r: &[u8]| String::from_utf8(present(&at, r, None).unwrap().1).unwrap();
        assert!(shown(b"20 text/gemini; charset=ISO-8859-1\r\n# caf\xe9").contains("<h1>caf\u{e9}</h1>"));
        assert!(shown(b"20 text/gemini;charset=\"windows-1252\"; lang=fr\r\n\x93q\x94").contains("\u{201c}q\u{201d}"));
        assert!(shown(b"20 text/plain; charset=koi8-r\r\n\xf0\xd2\xc9").contains("<pre>\u{41f}\u{440}\u{438}</pre>"));
        let (ty, page) = present(&at, b"20 text/html; charset=shift_jis\r\n<p>\x93\xfa</p>", None).unwrap();
        assert_eq!(
            (ty.as_str(), String::from_utf8(page).unwrap().as_str()),
            ("text/html; charset=utf-8", "<p>\u{65e5}</p>")
        );
        // No charset, or one that names no encoding, is UTF-8.
        assert!(shown(b"20 text/gemini\r\n# caf\xc3\xa9").contains("<h1>caf\u{e9}</h1>"));
        assert!(shown(b"20 text/plain; charset=nonsense\r\ncaf\xc3\xa9").contains("caf\u{e9}"));
        // A charset parameter on other media is left alone.
        assert_eq!(
            present(&at, b"20 image/png; charset=koi8-r\r\n\xf0", None).unwrap(),
            ("image/png".into(), b"\xf0".to_vec())
        );
    }

    #[test]
    fn a_byte_order_mark_is_not_part_of_the_text() {
        let at = url("gemini://h.test/");
        let shown = |r: &[u8]| String::from_utf8(present(&at, r, None).unwrap().1).unwrap();
        let page = shown(b"20 text/gemini\r\n\xef\xbb\xbf# Title");
        assert!(page.contains("<h1>Title</h1>") && !page.contains('\u{feff}'), "{page}");
        let page = shown(b"20 text/gemini; charset=utf-16le\r\n\xff\xfe#\x00 \x00x\x00");
        assert!(page.contains("<h1>x</h1>"), "{page}");
    }

    #[test]
    fn a_redirect_to_nowhere_is_not_followed() {
        let at = url("gemini://h.test/a");
        assert_eq!(followed(&at, b"30\r\n"), Ok(None));
        assert_eq!(followed(&at, b"31 \r\n"), Ok(None));
        let page = String::from_utf8(present(&at, b"30\r\n", None).unwrap().1).unwrap();
        assert!(page.contains("without saying where to"), "{page}");
    }

    #[test]
    fn other_statuses_are_pages_with_the_server_s_message() {
        let at = url("gemini://h.test/dir/");
        let shown = |r: &[u8]| String::from_utf8(present(&at, r, None).unwrap().1).unwrap();
        assert!(shown(b"10 Your name?\r\n").contains("<h1>Input requested (10)</h1>"));
        assert!(shown(b"51 <nope>\r\n").contains("<h1>Not found (51)</h1><p>&lt;nope&gt;</p>"));
        assert!(shown(b"42 x\r\n").contains("<h1>CGI error (42)</h1>"));
        assert!(shown(b"48 x\r\n").contains("<h1>Temporary failure (48)</h1>"));
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
        assert_eq!(first.certificates, [pinned("127.0.0.1", port, A_CERT)]);
        assert_eq!(first.peer_addr.map(|a| a.port()), Some(port));
        assert!(body(&first).contains("<h1>Hello</h1>"), "{}", body(&first));

        let (port, _server) = serve(&[A], b"20 text/gemini\r\nagain", true).await;
        let pin = CertPin::of(A_CERT);
        let again = fetch(&format!("gemini://127.0.0.1:{port}/"), Some(&pin)).await.unwrap();
        assert!(body(&again).contains("<p>again</p>"));
        // A pinned server's certificate is nothing new to record.
        assert_eq!(again.certificates, []);
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
        // What the user may choose to trust instead.
        assert_eq!(tls.fingerprint, Some(CertPin::of(B_CERT).digest()));
        assert_eq!(server.await.unwrap(), [None], "the request was sent");
    }

    #[tokio::test]
    async fn a_certificate_shown_without_its_key_is_refused_on_a_first_visit_too() {
        let (port, server) = serve(&[FORGED], b"20 text/gemini\r\nforged", true).await;
        let err = fetch(&format!("gemini://127.0.0.1:{port}/"), None).await.unwrap_err();
        let NetError::Tls(tls) = err else {
            panic!("not a TLS error: {err:?}")
        };
        assert!(
            tls.fingerprint.is_none(),
            "a forged certificate is offered for trusting"
        );
        assert_eq!(server.await.unwrap(), [None], "the request was sent");

        // Nor when it is the pinned one.
        let (port, _server) = serve(&[FORGED], b"20 text/gemini\r\nforged", true).await;
        let pin = CertPin::of(A_CERT);
        let err = fetch(&format!("gemini://127.0.0.1:{port}/"), Some(&pin))
            .await
            .unwrap_err();
        assert!(
            matches!(&err, NetError::Tls(tls) if tls.fingerprint.is_none()),
            "{err:?}"
        );
    }

    /// A changed certificate is offered for trusting only once the server proved it holds
    /// its key: a copy of another server's certificate is not.
    #[tokio::test]
    async fn a_changed_certificate_shown_without_its_key_is_not_offered_for_trusting() {
        let (port, server) = serve(&[FORGED], b"20 text/gemini\r\nforged", true).await;
        let pin = CertPin::of(B_CERT);
        let err = fetch(&format!("gemini://127.0.0.1:{port}/"), Some(&pin))
            .await
            .unwrap_err();
        let NetError::Tls(tls) = err else {
            panic!("not a TLS error: {err:?}")
        };
        assert!(tls.fingerprint.is_none(), "{tls:?}");
        assert!(!tls.message.contains("changed since the first visit"), "{tls:?}");
        assert_eq!(server.await.unwrap(), [None], "the request was sent");
    }

    #[test]
    fn input_is_asked_for_with_a_form_that_replaces_the_query() {
        let at = url("gemini://h.test/search?old#f");
        let shown = |r: &[u8]| String::from_utf8(present(&at, r, None).unwrap().1).unwrap();
        let page = shown(b"10 Search <for>\r\n");
        assert!(
            page.contains("<form method=\"get\" action=\"gemini://h.test/search\">"),
            "{page}"
        );
        assert!(
            page.contains("<label for=\"input\">Search &lt;for&gt;</label>"),
            "{page}"
        );
        assert!(page.contains("type=\"text\""), "{page}");
        assert!(shown(b"11 Password\r\n").contains("type=\"password\""));
        assert!(shown(b"10\r\n").contains("The server asks for input."));
    }

    #[test]
    fn a_single_field_s_input_is_the_query() {
        let data = |pairs: &[(&str, &str)]| -> Vec<(String, String)> {
            pairs.iter().map(|&(k, v)| (k.into(), v.into())).collect()
        };
        assert_eq!(
            Gemini.form_query(&data(&[("input", "a b/?#ü~")])).as_deref(),
            Some("a%20b%2F%3F%23%C3%BC~")
        );
        assert_eq!(Gemini.form_query(&data(&[("a", "1"), ("b", "2")])), None);
    }

    #[tokio::test]
    async fn a_redirect_on_the_same_server_is_followed() {
        let (port, server) = serve_each(
            &[
                (A, b"31 /b\r\n"),
                (A, b"30 c?x\r\n"),
                (A, b"20 text/gemini\r\n=> d D\r\n"),
            ],
            true,
        )
        .await;
        let response = fetch(&format!("gemini://127.0.0.1:{port}/a#top"), None).await.unwrap();
        let requests: Vec<_> = server
            .await
            .unwrap()
            .into_iter()
            .map(|l| String::from_utf8(l.unwrap()).unwrap())
            .collect();
        assert_eq!(
            requests,
            [
                format!("gemini://127.0.0.1:{port}/a\r\n"),
                format!("gemini://127.0.0.1:{port}/b\r\n"),
                format!("gemini://127.0.0.1:{port}/c?x\r\n"),
            ]
        );
        // The fragment the page was asked with stays, and is never sent.
        let at = format!("gemini://127.0.0.1:{port}/c?x#top");
        assert_eq!(response.final_url.as_ref().map(Url::as_str), Some(at.as_str()));
        // Links resolve against where the page ended up.
        assert!(
            body(&response).contains(&format!("href=\"gemini://127.0.0.1:{port}/d\"")),
            "{}",
            body(&response)
        );
        assert_eq!(response.certificates, [pinned("127.0.0.1", port, A_CERT)]);
    }

    #[tokio::test]
    async fn a_first_visit_s_redirect_is_held_to_its_first_certificate() {
        let (port, server) = serve_each(&[(A, b"30 /b\r\n"), (B, b"20 text/gemini\r\nswapped")], true).await;
        let err = fetch(&format!("gemini://127.0.0.1:{port}/a"), None).await.unwrap_err();
        // Nothing is pinned yet, so there is no pin to replace and nothing offered to trust.
        assert!(
            matches!(&err, NetError::Tls(tls) if tls.fingerprint.is_none() && tls.message.contains("redirect")),
            "{err:?}"
        );
        assert_eq!(server.await.unwrap()[1], None, "the second request was sent");
    }

    #[tokio::test]
    async fn redirects_stop_after_five() {
        let turns = [(A, &b"30 /again\r\n"[..]); MAX_REDIRECTS + 1];
        let (port, server) = serve_each(&turns, true).await;
        let response = fetch(&format!("gemini://127.0.0.1:{port}/"), None).await.unwrap();
        assert_eq!(server.await.unwrap().len(), MAX_REDIRECTS + 1);
        assert!(body(&response).contains("redirected 5 times"), "{}", body(&response));
        assert!(body(&response).contains(&format!("gemini://127.0.0.1:{port}/again")));
    }

    #[tokio::test]
    async fn a_redirect_to_another_server_is_followed_and_both_are_reported() {
        let (there, second) = serve(&[B], b"20 text/gemini\r\n# There\r\n", true).await;
        let redirect = format!("30 gemini://127.0.0.1:{there}/there\r\n");
        let (here, first) = serve(&[A], redirect.into_bytes().leak(), true).await;
        let response = fetch(&format!("gemini://127.0.0.1:{here}/"), None).await.unwrap();
        assert_eq!(first.await.unwrap().len(), 1);
        assert_eq!(
            second.await.unwrap()[0].as_deref().unwrap(),
            format!("gemini://127.0.0.1:{there}/there\r\n").as_bytes()
        );
        assert_eq!(
            response.final_url.as_ref().map(Url::as_str),
            Some(format!("gemini://127.0.0.1:{there}/there").as_str())
        );
        assert!(body(&response).contains("<h1>There</h1>"), "{}", body(&response));
        assert_eq!(
            response.certificates,
            [pinned("127.0.0.1", here, A_CERT), pinned("127.0.0.1", there, B_CERT)]
        );
    }

    #[tokio::test]
    async fn a_server_a_redirect_leads_to_is_held_to_its_own_pin() {
        let (there, second) = serve(&[B], b"20 text/gemini\r\nsecret", true).await;
        let redirect = format!("30 gemini://127.0.0.1:{there}/\r\n");
        let (here, _first) = serve(&[A], redirect.into_bytes().leak(), true).await;
        let url = Url::parse(&format!("gemini://127.0.0.1:{here}/")).unwrap();
        let pins = ZonePins(vec![pinned("127.0.0.1", there, A_CERT)]);
        let err = fetch_with(&url, &pins).await.unwrap_err();
        // Not the server the page was asked for: nothing is offered for trusting there.
        assert!(
            matches!(&err, NetError::Tls(tls) if tls.fingerprint.is_none() && tls.message.contains("visit it")),
            "{err:?}"
        );
        assert_eq!(second.await.unwrap(), [None], "the request was sent");
    }

    /// A failure further on does not lose what the servers before it showed.
    #[tokio::test]
    async fn a_redirect_that_fails_still_reports_the_first_visits_before_it() {
        let (there, _second) = serve(&[B], b"20 text/gemini\r\nsecret", true).await;
        let redirect = format!("30 gemini://127.0.0.1:{there}/\r\n");
        let (here, _first) = serve(&[A], redirect.into_bytes().leak(), true).await;
        let url = Url::parse(&format!("gemini://127.0.0.1:{here}/")).unwrap();
        let pins = ZonePins(vec![pinned("127.0.0.1", there, A_CERT)]);
        let err = failure(&url, &pins).await;
        assert!(matches!(err.error, NetError::Tls(_)), "{err:?}");
        assert_eq!(err.certificates, [pinned("127.0.0.1", here, A_CERT)]);
    }

    /// A server asking for a client certificate was reached: its certificate is pinned, so
    /// the visit that presents one goes to the same server.
    #[tokio::test]
    async fn a_server_asking_for_a_certificate_reports_its_own() {
        let (port, _server) = serve(&[A], b"60 Who are you?\r\n", true).await;
        let url = Url::parse(&format!("gemini://127.0.0.1:{port}/")).unwrap();
        let err = failure(&url, &ZonePins(Vec::new())).await;
        assert!(IdentityAsked::of(&err.error).is_some(), "{err:?}");
        assert_eq!(err.certificates, [pinned("127.0.0.1", port, A_CERT)]);
    }

    #[tokio::test]
    async fn a_redirect_to_another_scheme_is_a_link() {
        let (port, server) = serve(&[A], b"30 https://example.test/elsewhere\r\n", true).await;
        let response = fetch(&format!("gemini://127.0.0.1:{port}/"), None).await.unwrap();
        assert_eq!(server.await.unwrap().len(), 1);
        assert_eq!(response.final_url, None);
        assert!(
            body(&response).contains("<a href=\"https://example.test/elsewhere\">"),
            "{}",
            body(&response)
        );
    }

    #[tokio::test]
    async fn a_server_that_says_slow_down_is_asked_again_only_after_its_delay() {
        let (port, server) = serve_each(&[(A, b"44 Easy\r\n"), (A, b"20 text/gemini\r\nfine")], true).await;
        let at = format!("gemini://127.0.0.1:{port}/");
        let first = fetch(&at, None).await.unwrap();
        assert!(
            body(&first).contains("<h1>Slow down (44)</h1><p>Easy</p>") && body(&first).contains("in 1 second"),
            "{}",
            body(&first)
        );
        let asked = std::time::Instant::now();
        let second = fetch(&at, None).await.unwrap();
        assert!(
            asked.elapsed() >= slow_down::FIRST_DELAY - std::time::Duration::from_millis(50),
            "asked again after {:?}",
            asked.elapsed()
        );
        assert!(body(&second).contains("<p>fine</p>"), "{}", body(&second));
        assert_eq!(server.await.unwrap().len(), 2);
        // Answering ends the delay.
        assert_eq!(slow_down::waiting(&("127.0.0.1".into(), port)), None);
    }

    #[tokio::test]
    async fn a_request_that_would_wait_too_long_is_not_sent() {
        // Nothing listens there: the request must not get as far as connecting.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let server = ("127.0.0.1".to_string(), port);
        let mut delay = None;
        while delay.is_none_or(|delay| delay <= slow_down::LONGEST_WAIT) {
            delay = slow_down::answered(&server, true);
        }
        let response = fetch(&format!("gemini://127.0.0.1:{port}/"), None).await.unwrap();
        assert!(
            body(&response).contains("the request was not sent") && body(&response).contains("16 seconds"),
            "{}",
            body(&response)
        );
        assert_eq!(response.peer_addr, None);
    }

    fn identity_for(at: &str) -> ClientIdentity {
        ClientIdentity::generate(IdentityScope::for_url(&url(at)).unwrap(), "Visitor").unwrap()
    }

    async fn fetch_presenting(at: &str, identity: &ClientIdentity) -> Result<ProtocolResponse, NetError> {
        Gemini
            .fetch(ProtocolRequest {
                url: &url(at),
                reach: AddressSpace::Loopback,
                pins: None,
                identity: Some(identity),
            })
            .await
            .map_err(|e| e.error)
    }

    #[tokio::test]
    async fn a_server_asking_for_a_certificate_fails_the_navigation_with_its_message() {
        let (port, _server) = serve(&[A], b"60 Who are you?\r\n", true).await;
        let at = format!("gemini://127.0.0.1:{port}/private/x?q");
        let err = fetch(&at, None).await.unwrap_err();
        assert_eq!(
            IdentityAsked::of(&err),
            Some(&IdentityAsked {
                url: url(&at),
                status: 60,
                message: "Who are you?".into(),
                presented: false,
            }),
            "{err:?}"
        );
        assert!(matches!(
            crate::LoadError::from(&err),
            crate::LoadError::ClientCertificate {
                status: 60,
                presented: false,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn an_identity_is_presented_inside_its_scope_only() {
        let (port, server) = serve_tls(
            &[
                (A, b"31 /app/b\r\n"),
                (A, b"30 /other\r\n"),
                (A, b"20 text/gemini\r\nfine"),
            ],
            Tls::AskingClients,
            true,
        )
        .await;
        let identity = identity_for(&format!("gemini://127.0.0.1:{port}/app/"));
        let response = fetch_presenting(&format!("gemini://127.0.0.1:{port}/app/a"), &identity)
            .await
            .unwrap();
        assert!(body(&response).contains("<p>fine</p>"), "{}", body(&response));
        let seen: Vec<_> = server.await.unwrap().into_iter().map(Option::unwrap).collect();
        let line = |path: &str| format!("gemini://127.0.0.1:{port}{path}\r\n").into_bytes();
        assert_eq!(
            seen,
            [
                (line("/app/a"), Some(identity.certificate.clone())),
                (line("/app/b"), Some(identity.certificate.clone())),
                // Above the path the identity was made for: not presented.
                (line("/other"), None),
            ]
        );
    }

    #[tokio::test]
    async fn a_server_refusing_the_presented_certificate_says_so() {
        let (port, _server) = serve_tls(&[(A, b"61 Not you\r\n")], Tls::AskingClients, true).await;
        let at = format!("gemini://127.0.0.1:{port}/");
        let err = fetch_presenting(&at, &identity_for(&at)).await.unwrap_err();
        assert!(
            IdentityAsked::of(&err).is_some_and(|asked| asked.status == 61 && asked.presented),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn an_identity_never_goes_out_over_tls_1_2() {
        let (port, server) = serve_tls(&[(A, b"20 text/gemini\r\nfine")], Tls::AskingClientsOver12, true).await;
        let at = format!("gemini://127.0.0.1:{port}/");
        let err = fetch_presenting(&at, &identity_for(&at)).await.unwrap_err();
        assert!(
            matches!(&err, NetError::Tls(tls) if tls.message.contains("TLS 1.3 only")),
            "{err:?}"
        );
        assert_eq!(server.await.unwrap(), [None], "the handshake went through");
        // Without one, the same server is reached over 1.2.
        let (port, _server) = serve_tls(&[(A, b"20 text/gemini\r\nfine")], Tls::AskingClientsOver12, true).await;
        let response = fetch(&format!("gemini://127.0.0.1:{port}/"), None).await.unwrap();
        assert!(body(&response).contains("<p>fine</p>"));
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
                pins: None,
                identity: None,
            })
            .await
            .unwrap_err()
            .error;
        assert!(matches!(
            err,
            NetError::Blocked {
                reason: BlockReason::UrlPolicy,
                ..
            }
        ));
    }
}
