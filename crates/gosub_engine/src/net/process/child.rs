//! The network process: the only part of the engine that may open a socket.
//!
//! What only Linux can do - pass a ring fd for a streamed body, hold a direct
//! line to the cookie vault - lives in `platform`; the same API elsewhere
//! declines, so this file has no platform branches of its own.

use crate::net::emitter::null_emitter::NullEmitter;
use crate::net::fetcher::{Fetcher, FetcherConfig};
use crate::net::process::protocol::{
    flatten_headers, rebuild_headers, CookieScope, FetchOutcome, FromNet, NetEventWire, NetFetch, RequestTag, ToNet,
};
use crate::net::ssrf::AddressSpace;
use crate::net::types::{FetchRequest, FetchResult, RequestBody};
use gosub_ipc::Endpoint;
use http::Method;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use url::Url;

#[cfg(target_os = "linux")]
#[path = "child/linux.rs"]
mod platform;
#[cfg(not(target_os = "linux"))]
#[path = "child/portable.rs"]
mod platform;

use platform::{Streamed, VaultLink};

/// How long a shutdown drain waits for in-flight requests before giving up.
/// Shorter than the broker's `SHUTDOWN_GRACE`, so a draining child exits on
/// its own rather than being killed mid-drain.
const DRAIN_GRACE: std::time::Duration = std::time::Duration::from_secs(3);

/// Run as the network process until the broker disconnects or says to stop.
pub fn serve(link: Endpoint, vault: Option<Endpoint>) -> i32 {
    // A vault that stops answering must cost one request its cookies, not
    // wedge every request behind the mutex.
    let vault: Arc<Mutex<Option<VaultLink>>> = Arc::new(Mutex::new(vault.map(VaultLink::new)));
    gosub_sandbox::capture_process_title_region();
    gosub_sandbox::set_process_title("gosub-net", "gosub: network process");

    // Force glibc to load its NSS resolver modules *now*, while this process
    // may still map executable pages. `getaddrinfo` `dlopen`s `libnss_dns.so`
    // on first use, and the sandbox denies `mmap(PROT_EXEC)` - so a name
    // resolved after the lockdown kills the process on a syscall that looks
    // nothing like DNS. The name deliberately does not resolve: what matters
    // is the module load, not the answer. Same shape as the font warm-up in
    // the renderer: do the thing that needs the privilege before dropping it.
    {
        use std::net::ToSocketAddrs;
        let _ = "gosub-resolver-warmup.invalid:80".to_socket_addrs();
    }

    // Read-only, and only these: the resolver configuration and the trust store.
    // A network stack that cannot read them cannot resolve a name or verify a
    // certificate, so denying files outright (as a renderer is) is not an option
    // here - the paths are scoped instead. Before the runtime: Landlock binds
    // this thread and the threads created after it, and the runtime's workers,
    // where every response is parsed, would otherwise start unscoped.
    let paths = gosub_sandbox::net_filesystem_paths();
    let fs_allow: Vec<(&std::path::Path, bool)> = paths.iter().map(|p| (p.as_path(), false)).collect();
    gosub_sandbox::scope_net_filesystem(&fs_allow);

    // Built before the seccomp half, which covers its threads too (TSYNC):
    // building it makes syscalls the allowlist has no reason to carry.
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("[net] could not start a runtime: {e}");
            return 1;
        }
    };
    gosub_sandbox::lock_down_net();

    // Requests run concurrently: each Fetch is spawned onto the runtime and
    // replies through the shared writer, tagged, so a slow response never
    // holds up the ones behind it. The broker bounds how many are in flight.
    let (link_tx, mut link_rx) = link.split();
    let link_tx = Arc::new(Mutex::new(link_tx));
    // How much body each request in flight wants previewed, by tag; what the
    // observer answers when the response headers are in.
    let previews: Arc<Mutex<HashMap<RequestTag, usize>>> = Arc::new(Mutex::new(HashMap::new()));

    // The vault ticket of each request in flight that has one, by tag; what
    // the fetcher's cookie hooks ask the vault under, hop by hop.
    let tickets: Tickets = Arc::new(Mutex::new(HashMap::new()));

    // In-process, `EngineNetContext` turns the fetcher's events into engine
    // events and resolves request references against engine state. This
    // process holds no tab map, no jar, no event bus, so its observer sends
    // each event back over the link tagged for its request, and the broker's
    // own observer for that request takes it from there. Its cookie hooks
    // keep no jar either: they ask the vault, under the request's ticket.
    let build = |reach: AddressSpace| {
        Fetcher::new(
            crate::net::fetcher::reach_config(&FetcherConfig::default(), reach),
            Arc::new(NetProcessContext {
                reach,
                link_tx: Arc::clone(&link_tx),
                previews: Arc::clone(&previews),
                vault: Arc::clone(&vault),
                tickets: Arc::clone(&tickets),
            }),
        )
        .map(Arc::new)
    };
    // One fetcher per address space a document can be in (see `net::ssrf`):
    // loopback reaches anything the user navigates to, local refuses loopback,
    // public refuses both. The broker says which serves a request.
    let (loopback, local, public) = match (
        build(AddressSpace::Loopback),
        build(AddressSpace::Local),
        build(AddressSpace::Public),
    ) {
        (Ok(loopback), Ok(local), Ok(public)) => (loopback, local, public),
        (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => {
            eprintln!("[net] could not build the fetcher: {e}");
            return 1;
        }
    };

    let shutdown = CancellationToken::new();
    let running = (loopback.clone(), local.clone(), public.clone());
    let cancel = shutdown.clone();
    runtime.spawn(async move {
        tokio::join!(
            running.0.run(cancel.clone()),
            running.1.run(cancel.clone()),
            running.2.run(cancel)
        );
    });

    let cancels: Arc<Mutex<HashMap<RequestTag, CancellationToken>>> = Arc::new(Mutex::new(HashMap::new()));
    let tasks: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>> = Arc::new(Mutex::new(Vec::new()));

    // A read error ends the loop: it means the broker went away, which is a
    // normal end - the network process exists only to serve it.
    let mut drain = false;
    while let Ok(msg) = link_rx.recv::<ToNet>() {
        match msg {
            ToNet::Ping => {
                if link_tx.lock().send(&FromNet::Pong).is_err() {
                    break;
                }
            }
            ToNet::Shutdown => {
                drain = true;
                break;
            }
            ToNet::Cancel(tag) => {
                if let Some(token) = cancels.lock().remove(&tag) {
                    token.cancel();
                }
            }
            // On a runtime worker, not here: requests run there, so that is
            // where the confinement has to hold. The main thread proved
            // nothing about them once - it reported Landlock active while
            // the workers, started before it, ran without it.
            ToNet::Audit { tag } => {
                let link_tx = link_tx.clone();
                runtime.spawn(async move {
                    let report = platform::escape_audit();
                    let _ = link_tx.lock().send(&FromNet::Audit { tag, report });
                });
            }
            // The vault was respawned: its new line follows on the link.
            ToNet::VaultLine => match platform::adopt_vault_line(&mut link_rx) {
                Ok(line) => *vault.lock() = Some(line),
                Err(e) => eprintln!("[net] the new vault line did not arrive: {e}"),
            },
            ToNet::Fetch(fetch) => {
                let fetch = *fetch;
                let tag = fetch.tag;
                let token = CancellationToken::new();
                cancels.lock().insert(tag, token.clone());
                let previews = Arc::clone(&previews);
                let fetcher = match fetch.reach {
                    AddressSpace::Loopback => loopback.clone(),
                    AddressSpace::Local => local.clone(),
                    AddressSpace::Public => public.clone(),
                };
                let link_tx = link_tx.clone();
                let cancels = cancels.clone();
                let tickets = tickets.clone();
                let handle = runtime.spawn(async move {
                    let performed = perform(&fetcher, fetch, token, &tickets, &previews).await;
                    cancels.lock().remove(&tag);
                    match performed {
                        Performed::Done(outcome) => {
                            let mut link_tx = link_tx.lock();
                            let Some(outcome) = share_large_body(&mut link_tx, tag, outcome) else {
                                return;
                            };
                            if let Err(e) = link_tx.send(&FromNet::Reply { tag, outcome }) {
                                // A reply the link cannot carry (a body past the frame cap)
                                // is refused before any of it is written, so the link is
                                // still good: answer with an error rather than leave the
                                // broker waiting out its timeout for a reply that never
                                // comes. Any other write error means the broker went away,
                                // which the recv loop notices and ends the process on.
                                if e.kind() == std::io::ErrorKind::InvalidData {
                                    let outcome = FetchOutcome::Error(format!(
                                        "response too large to cross the process boundary: {e}"
                                    ));
                                    let _ = link_tx.send(&FromNet::Reply { tag, outcome });
                                }
                            }
                        }
                        Performed::Streaming(streamed) => streamed.deliver(tag, &link_tx).await,
                    }
                });
                let mut tasks = tasks.lock();
                tasks.retain(|h| !h.is_finished());
                tasks.push(handle);
            }
        }
    }

    // Shutdown promises to finish in-flight work (see `ToNet::Shutdown`): stop
    // reading (done - the loop ended), then flush what is still running so its
    // replies reach the broker. Bounded: the broker kills a child that lingers.
    if drain {
        let pending: Vec<_> = std::mem::take(&mut *tasks.lock());
        runtime.block_on(async {
            let _ = tokio::time::timeout(DRAIN_GRACE, futures_util::future::join_all(pending)).await;
        });
    }

    shutdown.cancel();
    0
}

/// Largest body a [`FetchOutcome::Ok`] carries in-band: half a frame, leaving
/// the head room. Past it the frame cannot be sent at all.
#[cfg(target_os = "linux")]
const MAX_IN_BAND_BODY: usize = gosub_ipc::MAX_FRAME_LEN as usize / 2;

/// Send a buffered body too large for one frame as [`FromNet::SharedReply`] and a
/// sealed memfd behind it, under the one hold of `link_tx` so nothing comes in
/// between. `None` once sent; otherwise the outcome to send as it is - in-band,
/// or, where no memfd can carry it, an error the link can.
#[cfg(target_os = "linux")]
fn share_large_body(
    link_tx: &mut gosub_ipc::EndpointTx,
    tag: RequestTag,
    outcome: FetchOutcome,
) -> Option<FetchOutcome> {
    let FetchOutcome::Ok {
        status,
        status_text,
        final_url,
        headers,
        body,
        peer_addr,
    } = outcome
    else {
        return Some(outcome);
    };
    if body.len() <= MAX_IN_BAND_BODY {
        return Some(FetchOutcome::Ok {
            status,
            status_text,
            final_url,
            headers,
            body,
            peer_addr,
        });
    }
    let len = body.len();
    let fd = match gosub_ipc::shm::create_sealed_blob(len, |buf| buf.copy_from_slice(&body)) {
        Ok(fd) => fd,
        Err(e) => {
            return Some(FetchOutcome::Error(format!(
                "a {len}-byte response cannot cross the process boundary: {e}"
            )))
        }
    };
    let head = FromNet::SharedReply {
        tag,
        status,
        status_text,
        final_url,
        headers,
        peer_addr,
        len: len as u64,
    };
    // A write error means the broker went away; the recv loop ends the process.
    if link_tx.send(&head).is_ok() {
        if let Err(e) = link_tx.send_fd(std::os::fd::AsRawFd::as_raw_fd(&fd)) {
            fd_never_followed(&e);
        }
    }
    None
}

/// A head that announced an fd went out and the fd did not. The broker's reader
/// now waits for an fd that will never come, then reads the next frame as one,
/// and no later message puts the link back in step. Closing this end does not
/// close the link (the recv loop holds the socket too), so the process ends:
/// the broker sees the link close and fails what was in flight.
#[cfg(target_os = "linux")]
fn fd_never_followed(e: &std::io::Error) -> ! {
    eprintln!("[net] an fd announced to the broker could not be sent ({e}); the link is out of step, exiting");
    std::process::exit(1)
}

/// No memfd to share a body through: every outcome goes as it is, and one past
/// the frame cap is answered with an error.
#[cfg(not(target_os = "linux"))]
fn share_large_body(_: &mut gosub_ipc::EndpointTx, _: RequestTag, outcome: FetchOutcome) -> Option<FetchOutcome> {
    Some(outcome)
}

/// The network process has no engine around it: no jar, no tabs. Its events
/// go back over the link to the broker's observer of the request; its cookie
/// hooks ask the vault under the request's ticket; what it does enforce
/// itself is the per-hop URL policy of its strict fetchers.
struct NetProcessContext {
    reach: AddressSpace,
    link_tx: Arc<Mutex<gosub_ipc::EndpointTx>>,
    previews: Arc<Mutex<HashMap<RequestTag, usize>>>,
    vault: Arc<Mutex<Option<VaultLink>>>,
    tickets: Tickets,
}

/// A request's vault ticket, and how the last `Set-Cookie` it handed the
/// vault went: `(url, stored)`. The last one is the final response's when
/// that had any, which decides whether the reply may drop its cookies.
struct TicketState {
    scope: CookieScope,
    last_store: Option<(String, bool)>,
}

type Tickets = Arc<Mutex<HashMap<RequestTag, TicketState>>>;

impl NetProcessContext {
    /// The ticket of the request `reference` names, if it has one.
    fn scope_of(&self, reference: gosub_sonar::RequestReference) -> Option<(RequestTag, CookieScope)> {
        let gosub_sonar::RequestReference::Tagged(tag) = reference else {
            return None;
        };
        let scope = self.tickets.lock().get(&tag)?.scope.clone();
        Some((tag, scope))
    }
}

/// The observer of one request in this process: each event the fetcher
/// reports goes over the link, tagged, to the broker.
struct LinkObserver {
    tag: RequestTag,
    link_tx: Arc<Mutex<gosub_ipc::EndpointTx>>,
    /// How much body the broker asked to see, if any.
    preview: Option<usize>,
    /// Bytes at the last progress event sent: the fetcher reports progress
    /// per read chunk, far too often to put each on the link.
    last_progress: std::sync::atomic::AtomicU64,
}

/// One progress event per this many bytes (and the one that completes the
/// body): the same rule the broker's emitter applies before the embedder.
const PROGRESS_STEP: u64 = 64 * 1024;

impl gosub_sonar::net::observer::NetObserver for LinkObserver {
    fn on_event(&self, event: crate::net::events::NetEvent) {
        use std::sync::atomic::Ordering;
        if let crate::net::events::NetEvent::Progress {
            received_bytes,
            expected_length,
            ..
        } = &event
        {
            let last = self.last_progress.load(Ordering::Relaxed);
            let complete = *expected_length == Some(*received_bytes);
            if received_bytes.saturating_sub(last) < PROGRESS_STEP && !complete {
                return;
            }
            self.last_progress.store(*received_bytes, Ordering::Relaxed);
        }
        let Some(event) = NetEventWire::from_net(&event) else {
            return;
        };
        // A link that no longer takes events no longer takes replies either;
        // the serve loop notices that and ends the process.
        let _ = self.link_tx.lock().send(&FromNet::Event { tag: self.tag, event });
    }

    fn body_capture_limit(&self, headers: &http::HeaderMap, content_length: Option<u64>) -> Option<usize> {
        self.preview
            .and_then(|cap| crate::net::emitter::capture_decision(true, cap, headers, content_length))
    }
}

impl gosub_sonar::net::fetcher_context::FetcherContext for NetProcessContext {
    fn observer_for(
        &self,
        reference: gosub_sonar::RequestReference,
        _: gosub_sonar::types::RequestId,
        _: gosub_sonar::net::types::ResourceKind,
        _: gosub_sonar::net::types::Initiator,
    ) -> Arc<dyn gosub_sonar::net::observer::NetObserver + Send + Sync> {
        // `perform` references every request by its tag; anything else the
        // fetcher asks about (nothing today) has nobody to report to.
        match reference {
            gosub_sonar::RequestReference::Tagged(tag) => Arc::new(LinkObserver {
                tag,
                link_tx: Arc::clone(&self.link_tx),
                preview: self.previews.lock().get(&tag).copied(),
                last_progress: std::sync::atomic::AtomicU64::new(0),
            }),
            _ => Arc::new(NullEmitter),
        }
    }
    fn on_ref_active(&self, _: gosub_sonar::RequestReference) {}
    fn on_ref_done(&self, _: gosub_sonar::RequestReference) {}

    // Asked at every hop. The vault answers each in a context it works out
    // from the grant itself and the chain it has seen, so a hop this process
    // made up buys nothing the page could not have had (see
    // `cookie_vault::child::serve_net`). The hop's method only narrows it.
    fn cookies_for_hop(
        &self,
        reference: gosub_sonar::RequestReference,
        hop: &gosub_sonar::CookieHop<'_>,
    ) -> Option<String> {
        let (_, scope) = self.scope_of(reference)?;
        tokio::task::block_in_place(|| {
            platform::vault_cookies(&self.vault, &scope, hop.url.as_str(), hop.method.is_safe())
        })
    }

    // Two tickets get the same answers from the vault when their grants say
    // the same: zone, document, context and URL. Only then may their
    // requests share a fetch, the leader's ticket asking for both.
    fn cookie_jar_key(&self, reference: gosub_sonar::RequestReference) -> String {
        match self.scope_of(reference) {
            Some((_, scope)) => format!(
                "{} {} {} {:?} {} {}",
                scope.zone,
                scope.top_level.as_deref().unwrap_or(""),
                scope.site.as_deref().unwrap_or(""),
                scope.samesite,
                scope.navigation,
                scope.url
            ),
            None => String::new(),
        }
    }

    fn on_cookies_received(&self, reference: gosub_sonar::RequestReference, url: &Url, values: &[&str]) {
        let Some((tag, scope)) = self.scope_of(reference) else {
            return;
        };
        let set_cookie = values.iter().map(|v| v.to_string()).collect();
        let stored =
            tokio::task::block_in_place(|| platform::vault_store(&self.vault, &scope, url.as_str(), set_cookie));
        if let Some(ticket) = self.tickets.lock().get_mut(&tag) {
            ticket.last_store = Some((url.to_string(), stored));
        }
    }

    fn is_url_allowed(&self, url: &Url) -> bool {
        if self.reach == AddressSpace::Loopback {
            return true;
        }
        match crate::net::ssrf::literal_verdict(url, self.reach) {
            Some(reason) => {
                eprintln!("[net] blocked {url}: {reason}");
                false
            }
            None => true,
        }
    }
}

/// What `perform` produced: a reply that travels whole, or a response head
/// whose body is still arriving and will follow it (see [`Streamed`]).
enum Performed {
    Done(FetchOutcome),
    Streaming(Streamed),
}

/// Perform one request and flatten the result to something that can travel.
async fn perform(
    fetcher: &Arc<Fetcher>,
    fetch: NetFetch,
    cancel: CancellationToken,
    tickets: &Tickets,
    previews: &Mutex<HashMap<RequestTag, usize>>,
) -> Performed {
    let streaming = fetch.streaming && platform::STREAMING;
    // Known before the fetcher asks its observer, taken back when the
    // request is over either way.
    let tag = fetch.tag;
    if let Some(cap) = fetch.body_preview {
        previews.lock().insert(tag, cap);
    }
    if let Some(scope) = fetch.cookies.clone() {
        tickets.lock().insert(
            tag,
            TicketState {
                scope,
                last_store: None,
            },
        );
    }
    let performed = perform_inner(fetcher, fetch, cancel, tickets, streaming).await;
    previews.lock().remove(&tag);
    tickets.lock().remove(&tag);
    performed
}

/// [`perform`] proper; split so the preview and ticket entries are removed on
/// every return.
async fn perform_inner(
    fetcher: &Arc<Fetcher>,
    fetch: NetFetch,
    cancel: CancellationToken,
    tickets: &Tickets,
    streaming: bool,
) -> Performed {
    let done = Performed::Done;
    // Cookies come from the vault, never from the broker, when this process
    // has its own line to it: the fetcher asks for them at every hop, under
    // the ticket (see `NetProcessContext::cookies_for`).
    let scope = fetch.cookies.clone();
    let fetch_tag = fetch.tag;
    let url = match Url::parse(&fetch.url) {
        Ok(u) => u,
        Err(e) => return done(FetchOutcome::Error(format!("bad url {}: {e}", fetch.url))),
    };
    let method = match Method::from_str(&fetch.method) {
        Ok(m) => m,
        Err(e) => return done(FetchOutcome::Error(format!("bad method {}: {e}", fetch.method))),
    };

    let mut headers = rebuild_headers(&fetch.headers);
    // Under a vault scope the broker's `Cookie` never counts: the vault's
    // answer for each hop does, or none at all. Without one, the broker
    // attached the cookies itself and they go as sent.
    if scope.is_some() {
        headers.remove(http::header::COOKIE);
    }

    // The tag is how this request's observer finds the link (see
    // `NetProcessContext::observer_for`).
    let builder = FetchRequest::builder(method, url)
        .with_reference(gosub_sonar::RequestReference::Tagged(fetch.tag))
        .with_headers(headers)
        .with_streaming(streaming)
        .with_auto_decode(true);
    let mut builder = fetch.context.apply(builder);
    if let Some(body) = fetch.body {
        // Plain bytes: the Content-Type already travelled in the headers.
        builder = builder.with_body(RequestBody::bytes(body));
    }
    let req = builder.build();

    let (tx, rx) = tokio::sync::oneshot::channel::<FetchResult>();
    fetcher.submit(req, cancel.clone(), tx).await;

    let result = tokio::select! {
        _ = cancel.cancelled() => return done(FetchOutcome::Error("cancelled by the broker".into())),
        r = rx => r,
    };
    // `Set-Cookie` went to the vault hop by hop, the final response's last;
    // the broker never sees it: the reply drops it once the vault has it. One
    // the vault did not take stays on the reply instead, and the broker stores
    // it through its own jar - the values pass the broker on that path, rather
    // than being lost. A final response whose cookies the fetcher never handed
    // over (credentials kept off it, or none it could read) has nothing the
    // vault should have taken.
    let vaulted = match (&scope, result.as_ref().ok().and_then(|r| r.meta())) {
        (Some(_), Some(meta)) => {
            let last = tickets.lock().get(&fetch_tag).and_then(|t| t.last_store.clone());
            match last {
                Some((url, stored)) if url == meta.final_url.as_str() => stored,
                _ => true,
            }
        }
        _ => false,
    };
    let reply_headers = |headers: &http::HeaderMap| reply_headers(headers, vaulted);
    match result {
        Ok(FetchResult::Buffered { meta, body }) => done(FetchOutcome::Ok {
            status: meta.status,
            status_text: meta.status_text,
            final_url: meta.final_url.to_string(),
            headers: reply_headers(&meta.headers),
            body: body.to_vec(),
            peer_addr: meta.peer_addr,
        }),
        Ok(FetchResult::Stream { meta, peek_buf, shared }) => {
            // What `Content-Length` promises past the peek, when it says.
            let expected = meta
                .headers
                .get(http::header::CONTENT_LENGTH)
                .and_then(|v| v.to_str().ok()?.trim().parse::<u64>().ok())
                .map(|len| len.saturating_sub(peek_buf.len() as u64));
            let head = FetchOutcome::Streaming {
                status: meta.status,
                status_text: meta.status_text,
                final_url: meta.final_url.to_string(),
                headers: reply_headers(&meta.headers),
                peek: peek_buf.as_ref().to_vec(),
                peer_addr: meta.peer_addr,
            };
            match platform::begin_stream(head, expected, shared) {
                Ok(streamed) => Performed::Streaming(streamed),
                Err(e) => done(FetchOutcome::Error(format!("could not set up a body stream: {e}"))),
            }
        }
        Ok(FetchResult::Error(e)) => done(FetchOutcome::Error(e.to_string())),
        Err(_) => done(FetchOutcome::Error("the fetcher dropped the request".into())),
    }
}

/// A response's headers for the reply to the broker. Once the vault has the
/// cookies, only what it was given leaves: a value it could not be sent stays,
/// for the broker's jar to judge.
fn reply_headers(headers: &http::HeaderMap, vaulted: bool) -> crate::net::process::protocol::HeaderList {
    let mut list = flatten_headers(headers);
    if vaulted {
        list.retain(|(name, value)| {
            !name.eq_ignore_ascii_case(http::header::SET_COOKIE.as_str()) || set_cookie_text(value).is_none()
        });
    }
    list
}

/// A `Set-Cookie` value as the vault takes it: text, read the way a cookie
/// jar reads it (UTF-8, so a non-ASCII value counts). Decides both what is
/// sent to the vault and what leaves the reply, so the two cannot disagree.
pub(crate) fn set_cookie_text(value: &[u8]) -> Option<&str> {
    std::str::from_utf8(value).ok()
}

#[cfg(test)]
mod reply_tests {
    use super::reply_headers;

    /// The vault is given every `Set-Cookie` that reads as text, a non-ASCII
    /// one included, and only those leave the reply; one that does not read
    /// stays, rather than being lost with the rest.
    #[test]
    fn only_what_the_vault_was_given_leaves_the_reply() {
        let mut headers = http::HeaderMap::new();
        headers.append(http::header::SET_COOKIE, "a=1".parse().unwrap());
        headers.append(
            http::header::SET_COOKIE,
            http::HeaderValue::from_bytes("b=caf\u{e9}".as_bytes()).unwrap(),
        );
        headers.append(
            http::header::SET_COOKIE,
            http::HeaderValue::from_bytes(b"c=\xff\xfe").unwrap(),
        );
        headers.append(http::header::CONTENT_TYPE, "text/html".parse().unwrap());

        let kept = reply_headers(&headers, true);
        assert_eq!(
            kept,
            vec![
                ("set-cookie".to_string(), b"c=\xff\xfe".to_vec()),
                ("content-type".to_string(), b"text/html".to_vec()),
            ]
        );
        assert_eq!(reply_headers(&headers, false).len(), 4, "not vaulted: all stay");
    }
}
