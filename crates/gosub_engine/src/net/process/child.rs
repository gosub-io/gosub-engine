//! The network process: the only part of the engine that may open a socket.
//!
//! What only Linux can do - pass a ring fd for a streamed body, hold a direct
//! line to the cookie vault - lives in `platform`; the same API elsewhere
//! declines, so this file has no platform branches of its own.

use crate::net::emitter::null_emitter::NullEmitter;
use crate::net::fetcher::{Fetcher, FetcherConfig};
use crate::net::process::protocol::{
    flatten_headers, rebuild_headers, FetchOutcome, FromNet, NetEventWire, NetFetch, RequestTag, ToNet,
};
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

    // In-process, `EngineNetContext` turns the fetcher's events into engine
    // events and resolves request references against engine state. This
    // process holds no tab map, no jar, no event bus, so its observer sends
    // each event back over the link tagged for its request, and the broker's
    // own observer for that request takes it from there. `cookies_for` must
    // stay silent: answering it would mean this process kept a jar.
    let build = |refuse_private: bool| {
        let cfg = if refuse_private {
            crate::net::fetcher::strict_config(&FetcherConfig::default())
        } else {
            FetcherConfig::default()
        };
        Fetcher::new(
            cfg,
            Arc::new(NetProcessContext {
                refuse_private,
                link_tx: Arc::clone(&link_tx),
                previews: Arc::clone(&previews),
            }),
        )
        .map(Arc::new)
    };
    // Two fetchers: one that may reach anything the user navigates to, and a
    // strict one for subresources of public documents (see `net::ssrf`); the
    // broker says which serves a request.
    let (fetcher, strict) = match (build(false), build(true)) {
        (Ok(f), Ok(s)) => (f, s),
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("[net] could not build the fetcher: {e}");
            return 1;
        }
    };

    let shutdown = CancellationToken::new();
    let (fetcher_run, strict_run) = (fetcher.clone(), strict.clone());
    let cancel = shutdown.clone();
    runtime.spawn(async move {
        tokio::join!(fetcher_run.run(cancel.clone()), strict_run.run(cancel));
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
                let fetcher = if fetch.refuse_private {
                    strict.clone()
                } else {
                    fetcher.clone()
                };
                let link_tx = link_tx.clone();
                let cancels = cancels.clone();
                let vault = vault.clone();
                let handle = runtime.spawn(async move {
                    let performed = perform(&fetcher, fetch, token, &vault, &previews).await;
                    cancels.lock().remove(&tag);
                    match performed {
                        Performed::Done(outcome) => {
                            let mut link_tx = link_tx.lock();
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

/// The network process has no engine around it: no cookies (the broker or
/// the vault attach those), no tabs. Its events go back over the link to
/// the broker's observer of the request; what it does enforce itself is the
/// per-hop URL policy of its strict fetcher.
struct NetProcessContext {
    refuse_private: bool,
    link_tx: Arc<Mutex<gosub_ipc::EndpointTx>>,
    previews: Arc<Mutex<HashMap<RequestTag, usize>>>,
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
    fn is_url_allowed(&self, url: &Url) -> bool {
        if !self.refuse_private {
            return true;
        }
        match crate::net::ssrf::literal_verdict(url) {
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
    vault: &Mutex<Option<VaultLink>>,
    previews: &Mutex<HashMap<RequestTag, usize>>,
) -> Performed {
    let streaming = fetch.streaming && platform::STREAMING;
    // Known before the fetcher asks its observer, taken back when the
    // request is over either way.
    let tag = fetch.tag;
    if let Some(cap) = fetch.body_preview {
        previews.lock().insert(tag, cap);
    }
    let performed = perform_inner(fetcher, fetch, cancel, vault, streaming).await;
    previews.lock().remove(&tag);
    performed
}

/// [`perform`] proper; split so the preview entry is removed on every return.
async fn perform_inner(
    fetcher: &Arc<Fetcher>,
    fetch: NetFetch,
    cancel: CancellationToken,
    vault: &Mutex<Option<VaultLink>>,
    streaming: bool,
) -> Performed {
    let done = Performed::Done;
    // Cookies come from the vault, never from the broker, when this process
    // has its own line to it. The scope is the broker's word on whose they are.
    let scope = fetch.cookies.clone();
    let cookie_header = match &scope {
        Some(scope) => tokio::task::block_in_place(|| platform::vault_cookies(vault, scope, &fetch.url)),
        None => None,
    };
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
    // answer for this request does, or none at all. Without one, the broker
    // attached the cookies itself and they go as sent.
    if scope.is_some() {
        headers.remove(http::header::COOKIE);
        if let Some(value) = cookie_header.as_deref().and_then(|v| v.parse().ok()) {
            headers.insert(http::header::COOKIE, value);
        }
    }

    // The tag is how this request's observer finds the link (see
    // `NetProcessContext::observer_for`).
    let mut builder = FetchRequest::builder(method, url)
        .with_reference(gosub_sonar::RequestReference::Tagged(fetch.tag))
        .with_headers(headers)
        .with_streaming(streaming)
        .with_auto_decode(true);
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
    // `Set-Cookie` goes to the vault from here; the broker never sees it.
    if let (Some(scope), Some(meta)) = (&scope, result.as_ref().ok().and_then(|r| r.meta())) {
        tokio::task::block_in_place(|| platform::vault_store(vault, scope, meta));
    }
    match result {
        Ok(FetchResult::Buffered { meta, body }) => done(FetchOutcome::Ok {
            status: meta.status,
            status_text: meta.status_text,
            final_url: meta.final_url.to_string(),
            headers: flatten_headers(&meta.headers),
            body: body.to_vec(),
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
                headers: flatten_headers(&meta.headers),
                peek: peek_buf.as_ref().to_vec(),
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
