//! `gopher://` (RFC 1436; URLs per RFC 4266), one of the engine's own [protocols].
//!
//! A request is one line on a TCP connection - the selector, CRLF - and the response is
//! everything the server sends until it closes. There is no status, no header and no length,
//! so the item type in the URL is all that says what the bytes are:
//! - menus (`1`) and search results (`7`) become an HTML page of links,
//! - text files (`0`) become a `<pre>` page,
//! - HTML (`h`) is served as it is,
//! - BinHex (`4`) and uuencoded (`6`) files are served decoded, under the name they carry
//!   ([`decode`]),
//! - the text-like extension types (`x`, `c`, `M`, `r`) are served raw as plain text,
//! - everything else is served raw under a sniffed media type, so images display and
//!   anything unknown goes to the download offer.
//!
//! A URL with a Gopher+ string (RFC 4266 §2.3) makes a Gopher+ request instead. Its reply
//! starts with a header saying where it ends, which is taken off before any of the above
//! ([`plus`]); an alternate view is shown as its view says, not by its item type.
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
//! A `+` line in a menu names a mirror of the item before it: the same item on another server
//! (RFC 1436 §3.8). It is shown as a link after the item, and a navigation from that menu tries
//! the item's mirrors in turn when its server cannot be connected to ([`mirrors`]). A server
//! that answered, if only with an error, is not left for a mirror. A mirror's page is served
//! under the mirror's URL, and each mirror is held to the reach and the ports as any server.
//!
//! [protocols]: crate::net::protocols

mod decode;
mod menu;
mod mirrors;
mod plus;
mod render;
mod request;
mod stream;

use crate::net::protocols::socket::{self, invalid, Close, Limits, Opened};
use crate::net::protocols::{BodyStream, Protocol, ProtocolError, ProtocolRequest, ProtocolResponse};
use crate::net::ssrf::AddressSpace;
use crate::net::types::NetError;
use async_trait::async_trait;
use bytes::Bytes;
use decode::{Decoding, Encoding};
use render::Lines;
use request::{Command, Request};
use std::net::SocketAddr;
use std::time::Duration;
use stream::PageStream;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::net::TcpStream;
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

    fn serves_mirrors(&self) -> bool {
        true
    }

    async fn fetch(&self, request: ProtocolRequest<'_>) -> Result<ProtocolResponse, ProtocolError> {
        Ok(fetch_within(request, &Limits::DEFAULT).await?)
    }
}

/// [`Gopher::fetch`] under `limits`.
async fn fetch_within(asked: ProtocolRequest<'_>, limits: &Limits) -> Result<ProtocolResponse, NetError> {
    let mut request = Request::from_url(asked.url).map_err(invalid)?;
    if request.needs_search_terms() {
        // The prompt needs no server, but it is placed by where its server is: the form
        // submits from it, and that search may reach only as far as the prompt's space.
        let addrs = socket::resolve(&Gopher, asked.url, &request.host, request.port, asked.reach).await?;
        return Ok(html(render::search_page(asked.url), socket::most_public(&addrs)));
    }
    // Only a plain request falls back on mirrors: a Gopher+ one asks the server it names what
    // that server says of the item, and a form's answers go to the server that asked.
    let plain = request.gopher_plus.is_none() && asked.body.is_none();
    let mirrors = match (asked.zone, asked.from) {
        (Some(zone), Some(from)) if plain => mirrors::of(zone, from, asked.url),
        _ => Vec::new(),
    };
    // A Gopher+ request's line is made before the item type gives way to a view's.
    let mut line = request.gopher_plus.is_some().then(|| request.wire_line());
    match request.command() {
        Some(Command::Ask) => {
            let server = connect(asked.url, request.clone(), asked.reach, &[]).await?;
            match ask(server, line.as_deref().unwrap_or_default(), limits, asked.body).await? {
                Ok(answers) => line = Some(answers),
                Err(page) => return Ok(page),
            }
        }
        Some(Command::Attributes(_) | Command::Directory(_)) => {
            let server = connect(asked.url, request.clone(), asked.reach, &[]).await?;
            return Ok(
                match attributes(server, line.as_deref().unwrap_or_default(), limits).await? {
                    Ok((body, peer)) => html(plus::attributes_page(asked.url, &request, &body), peer),
                    Err(page) => page,
                },
            );
        }
        Some(Command::View(view)) => {
            // The view says what the bytes are now, not the item type.
            request.item_type = view_type(view);
        }
        Some(Command::Item) | None => {}
    }
    let server = connect(asked.url, request, asked.reach, &mirrors).await?;
    let mirrored = (server.url != *asked.url).then(|| server.url.clone());
    let mut response = serve(server, line, asked.zone, limits).await?;
    // A mirror's page is the mirror's: its address, its history entry, its links' base.
    response.final_url = mirrored;
    Ok(response)
}

/// Longest the mirrors of an item may take to connect, all together, once its own server
/// did not answer. With the server's own [`socket::CONNECT_TIMEOUT`] for its name and its
/// connection, and [`socket::RESPONSE_TIMEOUT`], this stays inside the broker's wait for a
/// network-process reply.
const MIRRORS_TIMEOUT: Duration = Duration::from_secs(20);

/// A connection made for a request.
struct Connected {
    /// Of the server that answered: the request's own, or one of its mirrors.
    url: Url,
    request: Request,
    stream: TcpStream,
    peer: Option<SocketAddr>,
}

/// A connection for `request`, to the server `url` names or, when that one does not answer,
/// to the first of its `mirrors` that does. Only a connection that failed falls back: a
/// server refused by policy is refused, and a mirror refused by policy is skipped. Each
/// mirror is held to the reach and the blocked ports as its own URL.
async fn connect(url: &Url, request: Request, reach: AddressSpace, mirrors: &[Url]) -> Result<Connected, NetError> {
    let failed = match connect_to(url, &request, reach).await {
        Ok((stream, peer)) => {
            return Ok(Connected {
                url: url.clone(),
                request,
                stream,
                peer,
            })
        }
        Err(e @ NetError::Blocked { .. }) => return Err(e),
        Err(e) => e,
    };
    let deadline = tokio::time::Instant::now() + MIRRORS_TIMEOUT;
    for mirror in mirrors {
        let Ok(alternate) = Request::from_url(mirror) else {
            continue;
        };
        if alternate.item_type != request.item_type {
            continue;
        }
        match tokio::time::timeout_at(deadline, connect_to(mirror, &alternate, reach)).await {
            Ok(Ok((stream, peer))) => {
                log::info!("{url} did not answer ({failed}); serving it from its mirror {mirror}");
                return Ok(Connected {
                    url: mirror.clone(),
                    request: alternate,
                    stream,
                    peer,
                });
            }
            Ok(Err(e)) => log::debug!("mirror {mirror} of {url}: {e}"),
            Err(_) => break,
        }
    }
    Err(failed)
}

/// A connection to the server `url` names.
async fn connect_to(
    url: &Url,
    request: &Request,
    reach: AddressSpace,
) -> Result<(TcpStream, Option<SocketAddr>), NetError> {
    let to = socket::resolve(&Gopher, url, &request.host, request.port, reach).await?;
    socket::connect(url, &request.host, &to, reach).await
}

/// The response to a request on the connection made for it, its menus' mirrors remembered in
/// `zone`. `line` is what is sent when it is not the request's own wire line.
async fn serve(
    server: Connected,
    line: Option<Vec<u8>>,
    zone: Option<&str>,
    limits: &Limits,
) -> Result<ProtocolResponse, NetError> {
    let Connected {
        url,
        request,
        mut stream,
        peer,
    } = server;
    let url = &url;
    let line = line.unwrap_or_else(|| request.wire_line());
    if request.item_type == '3' && request.gopher_plus.is_none() {
        let body = socket::talk_within(&mut stream, &request.host, &line, Close::Clean, limits).await?;
        return Ok(present(&request, url, body, peer, zone));
    }
    let opened = socket::open_on(stream, peer, &request.host, &line, limits).await?;
    let opened = match block(opened, &request, url, limits).await? {
        Ok(opened) => opened,
        Err(page) => return Ok(page),
    };
    if let Some(encoding) = Encoding::of(request.item_type) {
        return decoded(&request, url, opened, limits, encoding).await;
    }
    let Opened { peek, rest, peer } = opened;
    let Some(rest) = rest else {
        return Ok(present(&request, url, peek, peer, zone));
    };
    if is_error_menu(&peek) {
        let body = socket::read_rest(peek, rest, &request.host, limits).await?;
        return Ok(present(&request, url, body, peer, zone));
    }
    let encoding = stream::encoding_for(&peek);
    let lines = match request.item_type {
        '1' | request::SEARCH_TYPE => {
            let lines = Lines::menu(url, encoding);
            Some(match zone {
                Some(zone) => lines.remembering_mirrors(zone, url),
                None => lines,
            })
        }
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
        None => (media_type(&request, &request.selector, &peek), peek, rest.reader),
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
        filename: None,
    })
}

/// A file sent as text, served decoded as it arrives: its first bytes, sniffed, and the rest
/// to stream. A file that does not decode within those first bytes is an error here; past
/// them, it fails the body.
async fn decoded(
    request: &Request,
    url: &Url,
    opened: Opened,
    limits: &Limits,
    encoding: Encoding,
) -> Result<ProtocolResponse, NetError> {
    let Opened { peek, rest, peer } = opened;
    if is_error_menu(&peek) {
        let body = match rest {
            Some(rest) => socket::read_rest(peek, rest, &request.host, limits).await?,
            None => peek,
        };
        return Ok(present(request, url, body, peer, None));
    }
    let encoded: Box<dyn AsyncRead + Send + Unpin> = match rest {
        Some(rest) => Box::new(std::io::Cursor::new(peek).chain(rest.reader)),
        None => Box::new(std::io::Cursor::new(peek)),
    };
    let mut reader = Decoding::new(encoding, encoded);
    let mut head = vec![0u8; socket::PEEK_BYTES];
    let mut len = 0;
    let mut ended = false;
    while len < head.len() {
        let n = tokio::time::timeout(limits.idle, reader.read(&mut head[len..]))
            .await
            .map_err(|_| NetError::Timeout(format!("waiting for {}", request.host)))?
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::InvalidData => invalid(format!("{url} does not decode: {e}")),
                _ => socket::io(e),
            })?;
        if n == 0 {
            ended = true;
            break;
        }
        len += n;
    }
    head.truncate(len);
    let filename = reader.name().map(|name| String::from_utf8_lossy(name).into_owned());
    Ok(ProtocolResponse {
        content_type: media_type(request, reader.name().unwrap_or_default(), &head).into(),
        body: Bytes::from(head),
        rest: (!ended).then(|| BodyStream {
            reader: Box::new(reader),
            idle: limits.idle,
        }),
        peer_addr: peer,
        certificates: Vec::new(),
        final_url: None,
        filename,
    })
}

/// The data block of a reply to a Gopher+ request, its header taken off and its end where the
/// header says ([`plus`]); a reply to any other request as it is. `Err` holds the page for a
/// Gopher+ error.
async fn block(
    opened: Opened,
    request: &Request,
    url: &Url,
    limits: &Limits,
) -> Result<Result<Opened, ProtocolResponse>, NetError> {
    if request.gopher_plus.is_none() {
        return Ok(Ok(opened));
    }
    let Opened { peek, rest, peer } = opened;
    let Some((header, len)) = plus::header(&peek, rest.is_none()).map_err(invalid)? else {
        return Ok(Ok(Opened { peek, rest, peer }));
    };
    let (peek, rest) = plus::limit(peek[len..].to_vec(), rest, header.length);
    if header.ok {
        return Ok(Ok(Opened { peek, rest, peer }));
    }
    let body = match rest {
        Some(rest) => socket::read_rest(peek, rest, &request.host, limits).await?,
        None => peek,
    };
    Ok(Err(html(
        render::error_page(url, &plus::error_text(&header, &body)),
        peer,
    )))
}

/// The attributes `line` asks for on `server`, read whole, and the peer they came from. `Err` holds the
/// page for an error, Gopher+ or a plain gopher error menu.
async fn attributes(
    server: Connected,
    line: &[u8],
    limits: &Limits,
) -> Result<Result<(Vec<u8>, Option<SocketAddr>), ProtocolResponse>, NetError> {
    let Connected {
        url,
        request,
        stream,
        peer,
    } = server;
    let (url, request) = (&url, &request);
    let opened = socket::open_on(stream, peer, &request.host, line, limits).await?;
    let Opened { peek, rest, peer } = match block(opened, request, url, limits).await? {
        Ok(opened) => opened,
        Err(page) => return Ok(Err(page)),
    };
    let body = match rest {
        Some(rest) => socket::read_rest(peek, rest, &request.host, limits).await?,
        None => peek,
    };
    if is_error_menu(&body) {
        return Ok(Err(html(render::error_page(url, &error_text(&body)), peer)));
    }
    Ok(Ok((body, peer)))
}

/// An item's +ASK form: its page, as `Err`, when nothing was `posted`; the request that sends
/// the answers when they were. The form is read again for the answers, so what is sent is
/// what the server asks, in its order.
async fn ask(
    server: Connected,
    line: &[u8],
    limits: &Limits,
    posted: Option<&[u8]>,
) -> Result<Result<Vec<u8>, ProtocolResponse>, NetError> {
    let (url, request) = (server.url.clone(), server.request.clone());
    let (url, request) = (&url, &request);
    let (body, peer) = match attributes(server, line, limits).await? {
        Ok(read) => read,
        Err(page) => return Ok(Err(page)),
    };
    let blocks = plus::blocks(&body);
    let Some(form) = blocks.iter().find(|block| block.name == b"ASK") else {
        let page = render::error_page(url, "This item has no Gopher+ form (+ASK).");
        return Ok(Err(html(page, peer)));
    };
    let questions = plus::questions(form);
    let Some(posted) = posted else {
        let encoding = menu::encoding_for(&body);
        let title = blocks
            .iter()
            .find(|block| block.name == b"INFO")
            .and_then(|info| menu::parse_line(encoding, info.value))
            .map(|item| item.display)
            .filter(|display| !display.is_empty());
        return Ok(Err(html(
            plus::form_page(url, title.as_deref(), &questions, encoding),
            peer,
        )));
    };
    match plus::answers(&questions, posted) {
        Ok(answers) => Ok(Ok(request.answer_lines(&answers))),
        Err(why) => Ok(Err(html(render::error_page(url, &why), peer))),
    }
}

/// The item type an alternate view is shown as. Only the views every Gopher+ server has
/// (Gopher+ A3.0) are pages; anything else is served raw and sniffed, as an item of no
/// particular type is, so a view the server calls `text/html` is no more active than a
/// text file.
fn view_type(view: &[u8]) -> char {
    let mime = view.split(|&b| b == b' ').next().unwrap_or_default();
    if mime.eq_ignore_ascii_case(b"text/plain") {
        '0'
    } else if mime.eq_ignore_ascii_case(b"application/gopher-menu")
        || mime.eq_ignore_ascii_case(b"application/gopher+-menu")
    {
        '1'
    } else {
        '9'
    }
}

/// What the response is, by the item type the URL asked for.
/// A menu's mirrors are remembered in `zone`.
fn present(
    request: &Request,
    url: &Url,
    body: Vec<u8>,
    peer: Option<SocketAddr>,
    zone: Option<&str>,
) -> ProtocolResponse {
    match request.item_type {
        '1' | request::SEARCH_TYPE => html(render::menu_page_in(url, &body, zone), peer),
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
            filename: None,
        },
        _ => ProtocolResponse {
            content_type: media_type(request, &request.selector, &body).into(),
            body: Bytes::from(body),
            rest: None,
            peer_addr: peer,
            certificates: Vec::new(),
            final_url: None,
            filename: None,
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

/// A media type for a binary item, from its first bytes, its item type and the extension of
/// `name` (the selector, or the name a decoded file carries). Gopher's binary item types do
/// not say which format they are, and `I` covers every image there is.
///
/// Only types that show inert or go to the download offer come out: images but SVG, audio,
/// video, PDF and archives. Never HTML or XML, which can carry script.
fn sniff(item_type: char, name: &[u8], body: &[u8]) -> &'static str {
    let images = matches!(item_type, 'I' | 'g' | 'p');
    let media = matches!(item_type, 's' | '<' | ';');
    let fits = |mime: &str| {
        let (top, sub) = mime.split_once('/').unwrap_or_default();
        let active = sub.contains("xml") || sub.contains("html");
        match top {
            _ if active => false,
            "image" => !media,
            "audio" | "video" => !images,
            "application" => !images && !media && SAFE_APPLICATION_TYPES.contains(&mime),
            _ => false,
        }
    };
    let sniffed = crate::net::decision::sniff_mime(body);
    if fits(sniffed) {
        return sniffed;
    }
    // A signature the detector does not know, as for MPEG audio without an ID3 tag: the
    // extension says, for audio and video only.
    match extension(name).and_then(|ext| MEDIA_EXTENSIONS.iter().find(|(e, _)| e.eq_ignore_ascii_case(ext))) {
        Some((_, mime)) if fits(mime) => mime,
        _ => "application/octet-stream",
    }
}

/// The application types a binary item is served as: PDF and the archives of WHATWG MIME
/// Sniffing.
const SAFE_APPLICATION_TYPES: &[&str] = &[
    "application/pdf",
    "application/zip",
    "application/gzip",
    "application/x-gzip",
    "application/vnd.rar",
    "application/x-rar-compressed",
];

/// Audio and video by file extension.
const MEDIA_EXTENSIONS: &[(&str, &str)] = &[
    ("aac", "audio/aac"),
    ("flac", "audio/flac"),
    ("m4a", "audio/mp4"),
    ("mid", "audio/midi"),
    ("midi", "audio/midi"),
    ("mp3", "audio/mpeg"),
    ("oga", "audio/ogg"),
    ("ogg", "audio/ogg"),
    ("opus", "audio/ogg"),
    ("wav", "audio/wav"),
    ("avi", "video/x-msvideo"),
    ("m4v", "video/mp4"),
    ("mkv", "video/x-matroska"),
    ("mov", "video/quicktime"),
    ("mp4", "video/mp4"),
    ("mpeg", "video/mpeg"),
    ("mpg", "video/mpeg"),
    ("ogv", "video/ogg"),
    ("webm", "video/webm"),
];

/// The extension of the last part of `name`.
fn extension(name: &[u8]) -> Option<&str> {
    let last = name.rsplit(|&b| b == b'/').next()?;
    let dot = last.iter().rposition(|&b| b == b'.')?;
    let ext = std::str::from_utf8(&last[dot + 1..]).ok()?;
    (!ext.is_empty()).then_some(ext)
}

/// The media type a streamed or decoded item is served as. The text-like extension types
/// (`x` XML, `c` calendar, `M` mail, `r` RTF) are plain text, shown escaped: XML served as
/// itself is parsed as a document, and can bring script along in the XHTML namespace.
fn media_type(request: &Request, name: &[u8], body: &[u8]) -> &'static str {
    match request.item_type {
        'x' | 'c' | 'M' | 'r' => "text/plain",
        item_type => sniff(item_type, name, body),
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
        filename: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::protocols::{self, socket::MAX_RESPONSE_BYTES};
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
        post(url, reach, None).await
    }

    /// [`fetch`] of a form posted with `body`.
    async fn post(url: &str, reach: AddressSpace, body: Option<&[u8]>) -> Result<ProtocolResponse, NetError> {
        let url = Url::parse(url).unwrap();
        Gopher
            .fetch(ProtocolRequest {
                url: &url,
                reach,
                pins: None,
                identity: None,
                zone: None,
                from: None,
                body,
            })
            .await
            .map_err(|e| e.error)
    }

    /// A Gopher+ server on loopback for a form: answers the first connection's request for
    /// attributes with `attributes`, reads the second's request through the end of its data
    /// block and answers it with `reply`. Hands back both requests.
    async fn serve_form(
        attributes: &'static [u8],
        reply: &'static [u8],
    ) -> (u16, tokio::task::JoinHandle<(Vec<u8>, Vec<u8>)>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let read_until = |end: &'static [u8]| {
                let listener = &listener;
                async move {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    let mut asked = Vec::new();
                    let mut byte = [0u8; 1];
                    while !asked.ends_with(end) && socket.read_exact(&mut byte).await.is_ok() {
                        asked.push(byte[0]);
                    }
                    (socket, asked)
                }
            };
            let (mut socket, first) = read_until(b"\r\n").await;
            socket.write_all(attributes).await.unwrap();
            drop(socket);
            let (mut socket, second) = read_until(b"\r\n.\r\n").await;
            socket.write_all(reply).await.unwrap();
            (first, second)
        });
        (port, task)
    }

    async fn fetch_under(url: &str, limits: &Limits) -> Result<ProtocolResponse, NetError> {
        let url = Url::parse(url).unwrap();
        let request = ProtocolRequest {
            url: &url,
            reach: AddressSpace::Loopback,
            pins: None,
            identity: None,
            zone: None,
            from: None,
            body: None,
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

    /// [`fetch`] in `zone`, as a navigation the page at `from` started.
    async fn fetch_from(url: &str, zone: &str, from: Option<&Url>) -> Result<ProtocolResponse, NetError> {
        post_from(url, zone, from, None).await
    }

    /// [`fetch_from`] of a form posted with `body`.
    async fn post_from(
        url: &str,
        zone: &str,
        from: Option<&Url>,
        body: Option<&[u8]>,
    ) -> Result<ProtocolResponse, NetError> {
        let url = Url::parse(url).unwrap();
        Gopher
            .fetch(ProtocolRequest {
                url: &url,
                reach: AddressSpace::Loopback,
                pins: None,
                identity: None,
                zone: Some(zone),
                from,
                body,
            })
            .await
            .map_err(|e| e.error)
    }

    /// A loopback port nothing listens on.
    fn refusing_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    }

    /// The menu at the URL returned, fetched in `zone`, listing `item` and then `after`.
    async fn menu_listing(zone: &str, item: String, after: &[String]) -> Url {
        let mut body = item + "\r\n";
        for line in after {
            body.push_str(line);
            body.push_str("\r\n");
        }
        body.push_str(".\r\n");
        let (port, _server) = serve_once(Box::leak(body.into_bytes().into_boxed_slice())).await;
        let menu = Url::parse(&format!("gopher://127.0.0.1:{port}/1/menu")).unwrap();
        let page = fetch_from(menu.as_str(), zone, None).await.unwrap();
        whole_body(page).await.unwrap();
        menu
    }

    #[tokio::test]
    async fn the_primary_refuses_and_the_mirror_serves() {
        let dead = refusing_port();
        let (mirror, server) = serve_once(b"from the mirror\r\n.\r\n").await;
        let menu = menu_listing(
            "zone-mirror-serves",
            format!("0Doc\t/doc\t127.0.0.1\t{dead}"),
            &[format!("+Doc\t/copy/doc\t127.0.0.1\t{mirror}")],
        )
        .await;
        let item = format!("gopher://127.0.0.1:{dead}/0/doc");
        let response = fetch_from(&item, "zone-mirror-serves", Some(&menu)).await.unwrap();
        assert_eq!(server.await.unwrap(), b"/copy/doc\r\n");
        let mirror_url = format!("gopher://127.0.0.1:{mirror}/0/copy/doc");
        assert_eq!(response.final_url.as_ref().map(Url::as_str), Some(mirror_url.as_str()));
        assert_eq!(response.peer_addr.map(|a| a.port()), Some(mirror));
        let page = whole_body(response).await.unwrap();
        assert!(std::str::from_utf8(&page).unwrap().contains("from the mirror"));
    }

    #[tokio::test]
    async fn a_posted_form_never_falls_back_on_a_mirror() {
        let dead = refusing_port();
        let (mirror, server) = serve_once(b"from the mirror\r\n.\r\n").await;
        let menu = menu_listing(
            "zone-mirror-post",
            format!("0Doc\t/doc\t127.0.0.1\t{dead}\t?"),
            &[format!("+Doc\t/copy/doc\t127.0.0.1\t{mirror}")],
        )
        .await;
        let item = format!("gopher://127.0.0.1:{dead}/0/doc");
        let err = post_from(&item, "zone-mirror-post", Some(&menu), Some(b"a0=secret"))
            .await
            .unwrap_err();
        assert!(!matches!(err, NetError::Blocked { .. }), "{err:?}");
        assert!(!server.is_finished(), "the mirror was asked");
        server.abort();
    }

    #[tokio::test]
    async fn a_mirror_s_menu_links_to_the_mirror() {
        let dead = refusing_port();
        let (mirror, _server) = serve_once(b"0Here\t/here\t\t\r\n.\r\n").await;
        let menu = menu_listing(
            "zone-mirror-menu",
            format!("1Dir\t/dir\t127.0.0.1\t{dead}"),
            &[format!("+Dir\t/dir\t127.0.0.1\t{mirror}")],
        )
        .await;
        let item = format!("gopher://127.0.0.1:{dead}/1/dir");
        let response = fetch_from(&item, "zone-mirror-menu", Some(&menu)).await.unwrap();
        let page = String::from_utf8(whole_body(response).await.unwrap().to_vec()).unwrap();
        assert!(page.contains(&format!("gopher://127.0.0.1:{mirror}/0/here")), "{page}");
    }

    #[tokio::test]
    async fn a_mirror_that_is_out_of_reach_is_skipped() {
        let dead = refusing_port();
        let (mirror, server) = serve_once(b"second\r\n.\r\n").await;
        let menu = menu_listing(
            "zone-mirror-skipped",
            format!("0Doc\t/doc\t127.0.0.1\t{dead}"),
            &[
                "+Doc\t/doc\t127.0.0.1\t25".to_string(),
                format!("+Doc\t/doc\t127.0.0.1\t{mirror}"),
            ],
        )
        .await;
        let item = format!("gopher://127.0.0.1:{dead}/0/doc");
        let response = fetch_from(&item, "zone-mirror-skipped", Some(&menu)).await.unwrap();
        assert_eq!(server.await.unwrap(), b"/doc\r\n");
        assert_eq!(response.peer_addr.map(|a| a.port()), Some(mirror));
    }

    #[tokio::test]
    async fn the_server_s_own_failure_stands_when_no_mirror_answers() {
        let dead = refusing_port();
        let menu = menu_listing(
            "zone-mirror-none",
            format!("0Doc\t/doc\t127.0.0.1\t{dead}"),
            &[
                "+Doc\t/doc\t127.0.0.1\t25".to_string(),
                format!("+Doc\t/doc\t127.0.0.1\t{}", refusing_port()),
            ],
        )
        .await;
        let item = format!("gopher://127.0.0.1:{dead}/0/doc");
        let err = fetch_from(&item, "zone-mirror-none", Some(&menu)).await.unwrap_err();
        assert!(matches!(err, NetError::Io(_)), "{err:?}");
        assert!(err.to_string().contains("refused"), "{err}");
    }

    #[tokio::test]
    async fn a_mirror_with_no_item_before_it_is_ignored() {
        let dead = refusing_port();
        let (mirror, _server) = serve_once(b"never\r\n.\r\n").await;
        let menu = menu_listing(
            "zone-mirror-orphan",
            format!("0Doc\t/doc\t127.0.0.1\t{dead}"),
            &["iWelcome\t\t\t".to_string(), format!("+Doc\t/doc\t127.0.0.1\t{mirror}")],
        )
        .await;
        let item = format!("gopher://127.0.0.1:{dead}/0/doc");
        let err = fetch_from(&item, "zone-mirror-orphan", Some(&menu)).await.unwrap_err();
        assert!(matches!(err, NetError::Io(_)), "{err:?}");
    }

    #[tokio::test]
    async fn mirrors_serve_only_a_navigation_from_their_menu_in_their_zone() {
        let dead = refusing_port();
        let (mirror, _server) = serve_once(b"never\r\n.\r\n").await;
        let menu = menu_listing(
            "zone-mirror-scoped",
            format!("0Doc\t/doc\t127.0.0.1\t{dead}"),
            &[format!("+Doc\t/doc\t127.0.0.1\t{mirror}")],
        )
        .await;
        let item = format!("gopher://127.0.0.1:{dead}/0/doc");
        let other = Url::parse("gopher://127.0.0.1/1/other").unwrap();
        for (zone, from) in [
            ("zone-mirror-scoped", None),
            ("zone-mirror-scoped", Some(&other)),
            ("another-zone", Some(&menu)),
        ] {
            let err = fetch_from(&item, zone, from).await.unwrap_err();
            assert!(matches!(err, NetError::Io(_)), "{zone} {from:?}: {err:?}");
        }
    }

    #[tokio::test]
    async fn a_server_that_answers_is_never_left_for_a_mirror() {
        // Connected and closed without a byte, or with an error menu: the server answered.
        for answer in [&b""[..], b"3Not here\t\terror.host\t1\r\n.\r\n"] {
            let (primary, _primary) = serve_once(answer).await;
            let (mirror, mirror_server) = serve_once(b"never\r\n.\r\n").await;
            let zone = format!("zone-mirror-answered-{}", answer.len());
            let menu = menu_listing(
                &zone,
                format!("0Doc\t/doc\t127.0.0.1\t{primary}"),
                &[format!("+Doc\t/doc\t127.0.0.1\t{mirror}")],
            )
            .await;
            let item = format!("gopher://127.0.0.1:{primary}/0/doc");
            let response = fetch_from(&item, &zone, Some(&menu)).await.unwrap();
            assert_eq!(response.final_url, None);
            assert_eq!(response.peer_addr.map(|a| a.port()), Some(primary));
            assert!(!mirror_server.is_finished());
            mirror_server.abort();
        }
    }

    #[tokio::test]
    async fn a_server_refused_by_policy_is_not_left_for_a_mirror() {
        let (mirror, _server) = serve_once(b"never\r\n.\r\n").await;
        let menu = menu_listing(
            "zone-mirror-policy",
            "0Doc\t/doc\t127.0.0.1\t25".to_string(),
            &[format!("+Doc\t/doc\t127.0.0.1\t{mirror}")],
        )
        .await;
        let err = fetch_from("gopher://127.0.0.1:25/0/doc", "zone-mirror-policy", Some(&menu))
            .await
            .unwrap_err();
        assert!(
            matches!(
                err,
                NetError::Blocked {
                    reason: BlockReason::BadPort,
                    ..
                }
            ),
            "{err:?}"
        );
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
        assert_eq!(page.matches("Line\n").count(), 200);
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
    async fn a_uuencoded_item_is_served_decoded_under_its_name() {
        let gif = b"GIF89a\x01\x00\x01\x00\x80\x00\x00";
        let text: &'static [u8] = Box::leak(decode::encode::uuencode("dot.gif", gif).into_boxed_slice());
        let (port, _server) = serve_once(text).await;
        let response = fetch(&format!("gopher://127.0.0.1:{port}/6/dot.uue"), AddressSpace::Loopback)
            .await
            .unwrap();
        assert_eq!(response.content_type, "image/gif");
        assert_eq!(response.filename.as_deref(), Some("dot.gif"));
        assert!(response.rest.is_none());
        assert_eq!(&response.body[..], gif);
    }

    #[tokio::test]
    async fn a_large_encoded_item_streams_decoded() {
        let data: Vec<u8> = (0..MAX_RESPONSE_BYTES + 4096).map(|i| (i % 251) as u8).collect();
        let text = decode::encode::binhex("big.bin", &data, b"fork", false);
        let text: &'static [u8] = Box::leak(text.into_boxed_slice());
        let (port, _server) = serve_once(text).await;
        let response = fetch(&format!("gopher://127.0.0.1:{port}/4/big.hqx"), AddressSpace::Loopback)
            .await
            .unwrap();
        assert!(response.rest.is_some(), "a file this size streams");
        assert_eq!(response.filename.as_deref(), Some("big.bin"));
        assert_eq!(whole_body(response).await.unwrap(), data);
    }

    #[tokio::test]
    async fn an_encoded_item_whose_crc_fails_at_the_end_fails_its_body() {
        let data = vec![b'x'; 64 * 1024];
        let text = decode::encode::binhex("bad.bin", &data, b"", true);
        let text: &'static [u8] = Box::leak(text.into_boxed_slice());
        let (port, _server) = serve_once(text).await;
        let response = fetch(&format!("gopher://127.0.0.1:{port}/4/bad.hqx"), AddressSpace::Loopback)
            .await
            .unwrap();
        let err = whole_body(response).await.unwrap_err();
        assert!(err.to_string().contains("CRC"), "{err}");
    }

    #[tokio::test]
    async fn an_item_that_does_not_decode_is_an_error() {
        let (port, _server) = serve_once(b"begin 644 a\r\nnot uuencode at all\r\n").await;
        let err = fetch(&format!("gopher://127.0.0.1:{port}/6/a"), AddressSpace::Loopback)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("does not decode"), "{err}");
    }

    #[tokio::test]
    async fn an_error_menu_for_an_encoded_item_is_an_error_page() {
        let (port, _server) = serve_once(b"3'/nope' does not exist\t\terror.host\t1\r\n.\r\n").await;
        let response = fetch(&format!("gopher://127.0.0.1:{port}/6/nope"), AddressSpace::Loopback)
            .await
            .unwrap();
        assert!(std::str::from_utf8(&response.body).unwrap().contains("does not exist"));
    }

    #[tokio::test]
    async fn a_selector_with_a_line_break_never_connects() {
        let err = fetch("gopher://127.0.0.1:6379/1/x%0D%0AFLUSHALL", AddressSpace::Loopback)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("forbidden byte"), "{err}");
    }

    #[tokio::test]
    async fn a_gopher_plus_item_loses_its_header_and_ends_at_its_length() {
        let (port, server) = serve_once(b"+12\r\nhello <world>\r\nnot part of it\r\n").await;
        let response = fetch(
            &format!("gopher://127.0.0.1:{port}/0/t.txt%09%09+"),
            AddressSpace::Loopback,
        )
        .await
        .unwrap();
        assert_eq!(server.await.unwrap(), b"/t.txt\t+\r\n");
        let page = String::from_utf8(whole_body(response).await.unwrap().to_vec()).unwrap();
        assert!(page.contains("<pre>hello &lt;world\n</pre>"), "{page}");
        assert!(!page.contains("+12"));
    }

    #[tokio::test]
    async fn a_gopher_plus_menu_split_inside_its_header_is_read() {
        let chunks = vec![
            b"+".to_vec(),
            b"-".to_vec(),
            b"1\r".to_vec(),
            b"\n1Sub\t/sub\t127.0.0.1\t70\t+\r\n.\r\n".to_vec(),
        ];
        let port = serve_slowly(chunks, Duration::from_millis(20), Duration::ZERO).await;
        let response = fetch(&format!("gopher://127.0.0.1:{port}/1/%09%09+"), AddressSpace::Loopback)
            .await
            .unwrap();
        let page = String::from_utf8(whole_body(response).await.unwrap().to_vec()).unwrap();
        assert!(page.contains(">Sub</a>"), "{page}");
        assert!(!page.contains("+-1"), "{page}");
    }

    #[tokio::test]
    async fn a_gopher_plus_binary_until_close_is_sniffed_without_its_header() {
        let (port, _server) = serve_once(b"+-2\r\nGIF89a\x01\x00\x01\x00").await;
        let response = fetch(
            &format!("gopher://127.0.0.1:{port}/I/pic%09%09+"),
            AddressSpace::Loopback,
        )
        .await
        .unwrap();
        assert_eq!(response.content_type, "image/gif");
        assert_eq!(&whole_body(response).await.unwrap()[..], b"GIF89a\x01\x00\x01\x00");
    }

    #[tokio::test]
    async fn a_counted_gopher_plus_binary_streams_its_length_and_no_more() {
        let len = 3 * socket::PEEK_BYTES + 7;
        let mut reply = format!("+{len}\r\n").into_bytes();
        reply.extend((0..len).map(|i| (i % 251) as u8));
        reply.extend(b"more than it said");
        let port = serve_slowly(vec![reply.clone()], Duration::ZERO, Duration::from_secs(10)).await;
        let response = fetch(
            &format!("gopher://127.0.0.1:{port}/9/big%09%09+"),
            AddressSpace::Loopback,
        )
        .await
        .unwrap();
        assert!(response.rest.is_some());
        let started = std::time::Instant::now();
        let body = whole_body(response).await.unwrap();
        assert!(started.elapsed() < Duration::from_secs(5), "waited for the close");
        assert_eq!(&body[..], &reply[reply.len() - len - 17..reply.len() - 17]);
    }

    #[tokio::test]
    async fn a_gopher_plus_error_is_an_error_page() {
        let (port, _server) = serve_once(b"--2\r\n1 Frodo <fng@bogus.edu>\r\n<b>no</b> such item\r\n").await;
        let response = fetch(&format!("gopher://127.0.0.1:{port}/0/x%09%09+"), AddressSpace::Loopback)
            .await
            .unwrap();
        let page = std::str::from_utf8(&response.body).unwrap();
        assert!(
            page.contains("class=\"e\">Item is not available.\nFrodo &lt;fng@bogus.edu&gt;\n&lt;b&gt;no&lt;/b&gt; such item</pre>"),
            "{page}"
        );
    }

    #[tokio::test]
    async fn a_dot_terminated_gopher_plus_error_is_read_to_its_dot() {
        // Past the peek, so the fetch does not wait for a close to have it.
        let mut reply = b"--1\r\n2 <admin@h>\r\nbusy\r\n".to_vec();
        reply.extend(b"x\r\n".repeat(socket::PEEK_BYTES / 3));
        reply.extend(b".\r\n");
        let port = serve_slowly(vec![reply], Duration::ZERO, Duration::from_secs(10)).await;
        let started = std::time::Instant::now();
        let response = fetch(&format!("gopher://127.0.0.1:{port}/0/x%09%09+"), AddressSpace::Loopback)
            .await
            .unwrap();
        assert!(started.elapsed() < Duration::from_secs(5));
        let page = std::str::from_utf8(&response.body).unwrap();
        assert!(page.contains("Try again later.\n&lt;admin@h&gt;\nbusy\nx\n"), "{page}");
        assert!(page.contains("x</pre>"), "{page}");
    }

    #[tokio::test]
    async fn a_plain_reply_to_a_gopher_plus_request_is_taken_whole() {
        let (port, _server) = serve_once(b"iOld server\t\t\t\r\n.\r\n").await;
        let response = fetch(&format!("gopher://127.0.0.1:{port}/1/%09%09+"), AddressSpace::Loopback)
            .await
            .unwrap();
        let page = String::from_utf8(whole_body(response).await.unwrap().to_vec()).unwrap();
        assert!(page.contains("Old server"), "{page}");
    }

    #[tokio::test]
    async fn an_alternate_view_is_shown_as_its_view_says() {
        let (port, server) = serve_once(b"+-2\r\n<p>a menu as text</p>\r\n").await;
        let response = fetch(
            &format!("gopher://127.0.0.1:{port}/1/dir%09%09+text/plain%20En_US"),
            AddressSpace::Loopback,
        )
        .await
        .unwrap();
        assert_eq!(server.await.unwrap(), b"/dir\t+text/plain En_US\r\n");
        let page = String::from_utf8(whole_body(response).await.unwrap().to_vec()).unwrap();
        assert!(
            page.contains("<pre>&lt;p&gt;a menu as text&lt;/p&gt;\n</pre>"),
            "{page}"
        );

        let (port, _server) = serve_once(b"+-2\r\n<!DOCTYPE html><script>alert(1)</script>").await;
        let response = fetch(
            &format!("gopher://127.0.0.1:{port}/0/doc%09%09+text/html"),
            AddressSpace::Loopback,
        )
        .await
        .unwrap();
        assert_eq!(response.content_type, "application/octet-stream");
    }

    #[tokio::test]
    async fn an_info_link_shows_the_item_s_attributes() {
        let (port, server) =
            serve_once(b"+-1\r\n+INFO: 0Doc\t/doc\t127.0.0.1\t70\t+\r\n+VIEWS:\r\n text/plain: <1k>\r\n.\r\n").await;
        let response = fetch(
            &format!("gopher://127.0.0.1:{port}/0/doc%09%09!"),
            AddressSpace::Loopback,
        )
        .await
        .unwrap();
        assert_eq!(server.await.unwrap(), b"/doc\t!\r\n");
        assert!(response.rest.is_none());
        let page = std::str::from_utf8(&response.body).unwrap();
        assert!(
            page.contains("<a href=\"gopher://127.0.0.1/0/doc%09%09+text/plain\">text/plain</a>"),
            "{page}"
        );
    }

    #[tokio::test]
    async fn a_directory_s_attributes_list_every_item() {
        let (port, server) = serve_once(
            b"+-2\r\n+INFO: 0A\t/a\th\t70\t+\r\n+VIEWS:\r\n text/plain:\r\n+INFO: 0B\t/b\th\t70\t+\r\n+VIEWS:\r\n application/pdf:\r\n",
        )
        .await;
        let response = fetch(
            &format!("gopher://127.0.0.1:{port}/1/dir%09%09$"),
            AddressSpace::Loopback,
        )
        .await
        .unwrap();
        assert_eq!(server.await.unwrap(), b"/dir\t$\r\n");
        let page = std::str::from_utf8(&response.body).unwrap();
        assert!(page.contains("href=\"gopher://h/0/a%09%09+text/plain\""), "{page}");
        assert!(page.contains("href=\"gopher://h/0/b%09%09+application/pdf\""), "{page}");
    }

    #[tokio::test]
    async fn attributes_a_plain_server_refuses_are_an_error_page() {
        let (port, _server) = serve_once(b"3'/doc\t!' does not exist\t\terror.host\t1\r\n.\r\n").await;
        let response = fetch(
            &format!("gopher://127.0.0.1:{port}/0/doc%09%09!"),
            AddressSpace::Loopback,
        )
        .await
        .unwrap();
        assert!(std::str::from_utf8(&response.body)
            .unwrap()
            .contains("class=\"e\">'/doc"));
    }

    const FORM: &[u8] = b"+-1\r\n+INFO: 0Shock <me>\t/form\t127.0.0.1\t70\t?\r\n+ASK:\r\n\
 Note: Answer <all>\r\n Ask: How many volts?\t230\r\n AskP: Password:\r\n AskL: Why?\r\n\
 Select: Really:1\r\n Select: Twice:0\r\n Choose: Now?\tYes\tNot!\r\n.\r\n";

    #[tokio::test]
    async fn a_form_link_shows_the_form_and_sends_nothing_else() {
        let (port, server) = serve_once(FORM).await;
        let url = format!("gopher://127.0.0.1:{port}/0/form%09%09%3F");
        let response = fetch(&url, AddressSpace::Loopback).await.unwrap();
        assert_eq!(server.await.unwrap(), b"/form\t!\r\n");
        let page = std::str::from_utf8(&response.body).unwrap();
        assert!(
            page.contains(&format!("<form method=\"post\" action=\"{url}\">")),
            "{page}"
        );
        assert!(page.contains("<h1>Shock &lt;me&gt;</h1>"), "{page}");
        assert!(page.contains("type=\"password\" name=\"a2\""), "{page}");
        assert!(page.contains("<button type=\"submit\">"), "{page}");
    }

    #[tokio::test]
    async fn a_posted_form_sends_its_answers_as_a_data_block() {
        let (port, server) = serve_form(FORM, b"+-2\r\nThe shock is <on> its way\r\n").await;
        let url = format!("gopher://127.0.0.1:{port}/0/form%09%09%3F");
        let form = b"a1=110&a2=s%C3%A9cret+pw&a3=line+one%0D%0Aline%09two%0D%0A&a4=1&a6=1";
        let refused = post(&url, AddressSpace::Loopback, Some(form)).await.unwrap();
        // The tab in the multi-line answer is refused before anything is sent.
        let page = String::from_utf8(refused.body.to_vec()).unwrap();
        assert!(page.contains("answer to question 4"), "{page}");
        assert!(!page.contains("cret"), "{page}");

        let (port, server2) = serve_form(FORM, b"+-2\r\nThe shock is <on> its way\r\n").await;
        let url = format!("gopher://127.0.0.1:{port}/0/form%09%09%3F");
        let form = b"a1=110&a2=s%C3%A9cret+pw&a3=line+one%0D%0Aline+two%0D%0A&a4=1&a6=1";
        let response = post(&url, AddressSpace::Loopback, Some(form)).await.unwrap();
        let (first, second) = server2.await.unwrap();
        assert_eq!(first, b"/form\t!\r\n");
        assert_eq!(
            second,
            "/form\t+\t1\r\n+-1\r\n110\r\ns\u{e9}cret pw\r\n2\r\nline one\r\nline two\r\n1\r\n0\r\nNot!\r\n.\r\n"
                .as_bytes()
        );
        let page = String::from_utf8(whole_body(response).await.unwrap().to_vec()).unwrap();
        assert!(page.contains("<pre>The shock is &lt;on&gt; its way\n</pre>"), "{page}");
        server.abort();
    }

    #[tokio::test]
    async fn a_form_that_wants_a_file_is_shown_but_never_sent() {
        let form: &'static [u8] = b"+-2\r\n+ASK:\r\n Ask: Name\r\n AskF: Save as\tout.txt\r\n";
        let (port, server) = serve_once(form).await;
        let url = format!("gopher://127.0.0.1:{port}/0/f%09%09%3F");
        let page = fetch(&url, AddressSpace::Loopback).await.unwrap();
        let page = std::str::from_utf8(&page.body).unwrap();
        assert!(
            page.contains("Save as <span class=\"t\">(AskF: not supported)</span>"),
            "{page}"
        );
        assert!(!page.contains("<button"), "{page}");
        server.await.unwrap();

        let (port, server) = serve_form(form, b"+-2\r\nsent\r\n").await;
        let url = format!("gopher://127.0.0.1:{port}/0/f%09%09%3F");
        let response = post(&url, AddressSpace::Loopback, Some(b"a0=me")).await.unwrap();
        let page = std::str::from_utf8(&response.body).unwrap();
        assert!(page.contains("asks for AskF, which is not supported"), "{page}");
        // Only the form was asked for.
        assert!(!server.is_finished());
        server.abort();
    }

    #[tokio::test]
    async fn an_item_without_a_form_says_so() {
        let (port, _server) = serve_once(b"+-2\r\n+INFO: 0X\t/x\th\t70\t+\r\n").await;
        let response = fetch(
            &format!("gopher://127.0.0.1:{port}/0/x%09%09%3F"),
            AddressSpace::Loopback,
        )
        .await
        .unwrap();
        assert!(std::str::from_utf8(&response.body).unwrap().contains("no Gopher+ form"));
    }

    #[test]
    fn only_the_standard_views_are_pages() {
        assert_eq!(view_type(b"Text/plain De_DE"), '0');
        assert_eq!(view_type(b"application/gopher+-menu"), '1');
        assert_eq!(view_type(b"application/gopher-menu"), '1');
        assert_eq!(view_type(b"text/html"), '9');
        assert_eq!(view_type(b"image/gif"), '9');
    }

    #[test]
    fn only_a_whole_error_line_is_an_error_menu() {
        assert!(is_error_menu(b"3Not found\t\terror.host\t1\r\n"));
        assert!(!is_error_menu(b"3\t\x00\x01binary"));
        assert!(!is_error_menu(b"1Menu\t/\th\t70\r\n"));
    }

    #[test]
    fn binary_types_are_sniffed() {
        let sniffed = |body: &[u8]| sniff('9', b"/file", body);
        assert_eq!(sniffed(b"\x89PNG\r\n\x1a\nrest"), "image/png");
        assert_eq!(sniffed(b"\xff\xd8\xff\xe0"), "image/jpeg");
        assert_eq!(sniffed(b"GIF89a\x01\x00\x01\x00"), "image/gif");
        assert_eq!(sniffed(b"RIFF\0\0\0\0WEBPVP8 "), "image/webp");
        assert_eq!(sniffed(b"%PDF-1.7"), "application/pdf");
        assert_eq!(sniffed(b"PK\x03\x04\x14\0\0\0\0\0"), "application/zip");
        assert_eq!(sniffed(b"\x1f\x8b\x08\0\0\0\0\0"), "application/gzip");
        assert_eq!(sniffed(b"\0\x01\x02\x03"), "application/octet-stream");
    }

    #[test]
    fn audio_and_video_are_sniffed() {
        let sniffed = |body: &[u8]| sniff('s', b"/sound", body);
        assert_eq!(sniffed(b"ID3\x03\0\0\0\0\x0fTIT2\0\0\0\x0f\0\0Title"), "audio/mpeg");
        assert_eq!(
            sniffed(b"OggS\0\x02\0\0\0\0\0\0\0\0\x01\x02\x03\x04\0\0\0\0\0\0\0\0\x01\x1e\x01vorbis"),
            "audio/ogg"
        );
        assert!(sniffed(b"RIFF\x24\0\0\0WAVEfmt \x10\0\0\0").contains("wav"));
        let mp4 = b"\0\0\0\x18ftypmp42\0\0\0\0mp42isom";
        assert_eq!(sniff(';', b"/clip", mp4), "video/mp4");
    }

    #[test]
    fn the_item_type_narrows_what_is_sniffed() {
        let mp3 = b"ID3\x03\0\0\0\0\x0fTIT2\0\0\0\x0f\0\0Title";
        assert_eq!(sniff('I', b"/pic", mp3), "application/octet-stream");
        assert_eq!(
            sniff('s', b"/snd", b"\x89PNG\r\n\x1a\nrest"),
            "application/octet-stream"
        );
        assert_eq!(sniff('s', b"/snd", b"%PDF-1.7"), "application/octet-stream");
    }

    #[test]
    fn the_extension_names_media_the_bytes_do_not() {
        let unknown = b"\0\x01\x02\x03";
        assert_eq!(sniff('s', b"/music/Song.MP3", unknown), "audio/mpeg");
        assert_eq!(sniff('9', b"/films/a.b/clip.webm", unknown), "video/webm");
        assert_eq!(sniff(';', b"/a.mp3/clip", unknown), "application/octet-stream");
        // An image item is no sound, whatever its name.
        assert_eq!(sniff('I', b"/song.mp3", unknown), "application/octet-stream");
        // Nor does a name make anything but media.
        assert_eq!(sniff('9', b"/page.html", unknown), "application/octet-stream");
    }

    #[test]
    fn nothing_is_sniffed_into_an_active_type() {
        let active: [&[u8]; 4] = [
            b"<!DOCTYPE html><html><script>alert(1)</script></html>",
            b"<svg xmlns=\"http://www.w3.org/2000/svg\"><script>alert(1)</script></svg>",
            b"<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"></html>",
            b"<?xml version=\"1.0\"?><rss version=\"2.0\"></rss>",
        ];
        for body in active {
            for item_type in ['I', '9', 's', ';', 'd', 'P', '4', '6'] {
                let mime = sniff(item_type, b"/x.svg", body);
                assert_eq!(
                    mime,
                    "application/octet-stream",
                    "{item_type} {:?}",
                    String::from_utf8_lossy(body)
                );
            }
        }
    }

    #[tokio::test]
    async fn a_text_like_item_is_plain_text() {
        let xml: &'static [u8] = b"<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"></html>";
        for item_type in ['x', 'c', 'M', 'r'] {
            let (port, _server) = serve_once(xml).await;
            let response = fetch(
                &format!("gopher://127.0.0.1:{port}/{item_type}/doc"),
                AddressSpace::Loopback,
            )
            .await
            .unwrap();
            assert_eq!(response.content_type, "text/plain");
            assert_eq!(&response.body[..], xml);
        }
    }

    #[tokio::test]
    async fn a_decoded_file_is_sniffed_with_its_own_name() {
        let text = decode::encode::uuencode("tune.mp3", &[0, 1, 2, 3]);
        let text: &'static [u8] = Box::leak(text.into_boxed_slice());
        let (port, _server) = serve_once(text).await;
        let response = fetch(&format!("gopher://127.0.0.1:{port}/6/tune.uue"), AddressSpace::Loopback)
            .await
            .unwrap();
        assert_eq!(response.content_type, "audio/mpeg");
    }
}
