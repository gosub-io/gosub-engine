//! The network process: the only part of the engine that may open a socket.

use crate::net::fetcher::{Fetcher, FetcherConfig};
use crate::net::process::protocol::{FetchOutcome, FromNet, NetFetch, RequestTag, ToNet};
use crate::net::types::{FetchRequest, FetchResult, RequestBody};
use gosub_ipc::Endpoint;
use http::Method;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use url::Url;

/// How long a shutdown drain waits for in-flight requests before giving up.
/// Shorter than the broker's `SHUTDOWN_GRACE`, so a draining child exits on
/// its own rather than being killed mid-drain.
const DRAIN_GRACE: std::time::Duration = std::time::Duration::from_secs(3);

/// Run as the network process until the broker disconnects or says to stop.
pub fn serve(link: Endpoint, vault: Option<Endpoint>) -> i32 {
    // A vault that stops answering must cost one request its cookies, not
    // wedge every request behind the mutex.
    let vault: Option<Arc<Mutex<VaultLink>>> = vault.map(|mut ep| {
        let _ = ep.rx.set_read_timeout(Some(VAULT_TIMEOUT));
        let _ = ep.tx.set_write_timeout(Some(VAULT_TIMEOUT));
        Arc::new(Mutex::new(VaultLink { link: ep, next_tag: 1 }))
    });
    gosub_sandbox::capture_process_title_region();
    gosub_sandbox::set_process_title("gosub-net", "gosub: network process");

    // Built before lockdown: spawning threads is not on the allowlist, so a
    // runtime created afterwards could not start its workers.
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("[net] could not start a runtime: {e}");
            return 1;
        }
    };

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
    // here - the paths are scoped instead.
    let paths = gosub_sandbox::net_filesystem_paths();
    let fs_allow: Vec<(&std::path::Path, bool)> = paths.iter().map(|p| (p.as_path(), false)).collect();
    gosub_sandbox::lock_down_net(&fs_allow);

    // No hooks: in-process, `EngineNetContext` turns these into engine events and
    // resolves request references against engine state. Here there is no engine to
    // resolve against - this process holds no tab map, no jar, no event bus - so
    // progress reporting stays the broker's job. `cookies_for` in particular must
    // stay silent: answering it would mean this process kept a jar.
    let build = |refuse_private: bool| {
        let cfg = if refuse_private {
            crate::net::fetcher::strict_config(&FetcherConfig::default())
        } else {
            FetcherConfig::default()
        };
        Fetcher::new(cfg, Arc::new(NetProcessContext { refuse_private })).map(Arc::new)
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

    // Requests run concurrently: each Fetch is spawned onto the runtime and
    // replies through the shared writer, tagged, so a slow response never
    // holds up the ones behind it. The broker bounds how many are in flight.
    let (link_tx, mut link_rx) = link.split();
    let link_tx = Arc::new(Mutex::new(link_tx));
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
            ToNet::Fetch(fetch) => {
                let tag = fetch.tag;
                let token = CancellationToken::new();
                cancels.lock().insert(tag, token.clone());
                let fetcher = if fetch.refuse_private {
                    strict.clone()
                } else {
                    fetcher.clone()
                };
                let link_tx = link_tx.clone();
                let cancels = cancels.clone();
                let vault = vault.clone();
                let handle = runtime.spawn(async move {
                    let performed = perform(&fetcher, fetch, token, vault.as_deref()).await;
                    cancels.lock().remove(&tag);
                    // A write error means the broker went away; the recv loop
                    // notices the same and ends the process.
                    match performed {
                        Performed::Done(outcome) => {
                            let _ = link_tx.lock().send(&FromNet::Reply { tag, outcome });
                        }
                        Performed::Streaming {
                            outcome,
                            ring,
                            producer,
                            chunks,
                        } => {
                            // Head and ring fd back to back, under one lock, so
                            // nothing else on the link comes between them.
                            {
                                use std::os::fd::AsRawFd as _;
                                let mut tx = link_tx.lock();
                                if tx.send(&FromNet::Reply { tag, outcome }).is_err()
                                    || tx.send_fd(ring.as_raw_fd()).is_err()
                                {
                                    return;
                                }
                            }
                            drop(ring); // the broker holds its duplicate; the mapping keeps ours
                            pump(producer, chunks).await;
                        }
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

/// The network process has no engine around it: no events, no cookies (the
/// broker attaches those). What it does enforce is the per-hop URL policy of
/// its strict fetcher.
struct NetProcessContext {
    refuse_private: bool,
}

impl gosub_sonar::net::fetcher_context::FetcherContext for NetProcessContext {
    fn observer_for(
        &self,
        _: gosub_sonar::RequestReference,
        _: gosub_sonar::types::RequestId,
        _: gosub_sonar::net::types::ResourceKind,
        _: gosub_sonar::net::types::Initiator,
    ) -> Arc<dyn gosub_sonar::net::observer::NetObserver + Send + Sync> {
        Arc::new(crate::net::emitter::null_emitter::NullEmitter)
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

/// Ring window for streamed bodies. Small on purpose: a large body wraps
/// through it many times, and neither side ever holds more than this (plus one
/// chunk) for the transport.
const RING_CAPACITY: u32 = 256 * 1024;

/// What `perform` produced: a reply that travels whole, or a response head
/// whose body is still arriving and will be pumped through a ring.
enum Performed {
    Done(FetchOutcome),
    Streaming {
        outcome: FetchOutcome,
        ring: std::os::fd::OwnedFd,
        producer: gosub_ipc::ring::RingProducer,
        /// Subscribed the moment the response arrived: the body has no
        /// replay, so a later subscription would miss its first chunks.
        chunks: BodyChunks,
    },
}

type BodyChunks = futures_util::stream::BoxStream<'static, Result<bytes::Bytes, gosub_sonar::net::types::NetError>>;

/// How many chunks the pump may fall behind the fetcher before the body drops
/// it as a slow subscriber. The ring's backpressure stalls the pump whenever
/// the broker is not draining; this queue absorbs that stall (16 KiB chunks:
/// at most 16 MiB held) so backpressure costs memory here, never bytes.
const PUMP_QUEUE: usize = 1024;

/// Move a body from the fetcher into the ring as it arrives. The producer's
/// `write_all` blocks (bounded) when the ring is full - the backpressure that
/// keeps this process from buffering - so it runs off the async worker. An
/// error from the fetcher, or a consumer that stopped draining, abandons the
/// stream: the producer drops unfinished and the consumer sees an abort.
async fn pump(mut producer: gosub_ipc::ring::RingProducer, mut chunks: BodyChunks) {
    use futures_util::StreamExt as _;
    while let Some(chunk) = chunks.next().await {
        let Ok(chunk) = chunk else {
            return;
        };
        if tokio::task::block_in_place(|| producer.write_all(&chunk)).is_err() {
            return;
        }
    }
    producer.finish();
}

const VAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// The direct line to the vault: one request in flight at a time, each
/// answer checked against the tag it was asked with.
struct VaultLink {
    link: Endpoint,
    next_tag: u64,
}

/// The `Cookie` header for a request, from the vault. Any failure - timeout,
/// a reply for another tag - is "no cookies"; a following request starts clean.
fn vault_cookies(
    vault: &Mutex<VaultLink>,
    scope: &crate::net::process::protocol::CookieScope,
    url: &str,
) -> Option<String> {
    use crate::cookie_vault::protocol::{FromVault, ToVault};
    let mut vault = vault.lock();
    let tag = vault.next_tag;
    vault.next_tag += 1;
    vault
        .link
        .send(&ToVault::Get {
            tag,
            scope: scope.clone(),
            url: url.to_string(),
            visible_only: false,
        })
        .ok()?;
    // Replies for an earlier, timed-out request are skipped, not answered with.
    loop {
        match vault.link.recv::<FromVault>().ok()? {
            FromVault::Cookies { tag: got, header } if got == tag => return header,
            FromVault::Cookies { tag: got, .. } if got < tag => continue,
            _ => return None,
        }
    }
}

/// Hand a response's `Set-Cookie` headers to the vault.
fn vault_store(
    vault: &Mutex<VaultLink>,
    scope: &crate::net::process::protocol::CookieScope,
    meta: &gosub_sonar::net::types::FetchResultMeta,
) {
    use crate::cookie_vault::protocol::ToVault;
    let set_cookie: Vec<String> = meta
        .headers
        .get_all(http::header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok().map(str::to_string))
        .collect();
    if set_cookie.is_empty() {
        return;
    }
    let _ = vault.lock().link.send(&ToVault::Store {
        zone: scope.zone.clone(),
        url: meta.final_url.to_string(),
        top_level: scope.top_level.clone(),
        set_cookie,
    });
}

/// Perform one request and flatten the result to something that can travel.
async fn perform(
    fetcher: &Arc<Fetcher>,
    fetch: NetFetch,
    cancel: CancellationToken,
    vault: Option<&Mutex<VaultLink>>,
) -> Performed {
    // A ring fd can only travel where the link carries fds.
    let streaming = fetch.streaming && cfg!(target_os = "linux");
    let done = Performed::Done;
    // Cookies come from the vault, never from the broker, when this process
    // has its own line to it. The scope is the broker's word on whose they are.
    let scope = fetch.cookies.clone();
    let cookie_header = match (vault, &scope) {
        (Some(vault), Some(scope)) => tokio::task::block_in_place(|| vault_cookies(vault, scope, &fetch.url)),
        _ => None,
    };
    let url = match Url::parse(&fetch.url) {
        Ok(u) => u,
        Err(e) => return done(FetchOutcome::Error(format!("bad url {}: {e}", fetch.url))),
    };
    let method = match Method::from_str(&fetch.method) {
        Ok(m) => m,
        Err(e) => return done(FetchOutcome::Error(format!("bad method {}: {e}", fetch.method))),
    };

    let mut headers = http::HeaderMap::new();
    for (name, value) in &fetch.headers {
        let parsed = http::header::HeaderName::from_str(name).ok().zip(value.parse().ok());
        if let Some((name, value)) = parsed {
            headers.insert(name, value);
        }
    }
    headers.remove(http::header::COOKIE);
    if let Some(value) = cookie_header.as_deref().and_then(|v| v.parse().ok()) {
        headers.insert(http::header::COOKIE, value);
    }

    let mut builder = FetchRequest::builder(method, url)
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
    let flat_headers = |headers: &http::HeaderMap| -> Vec<(String, String)> {
        headers
            .iter()
            .filter_map(|(n, v)| v.to_str().ok().map(|v| (n.as_str().to_string(), v.to_string())))
            .collect()
    };
    // `Set-Cookie` goes to the vault from here; the broker never sees it.
    if let (Some(vault), Some(scope), Some(meta)) = (vault, &scope, result.as_ref().ok().and_then(|r| r.meta())) {
        vault_store(vault, scope, meta);
    }
    match result {
        Ok(FetchResult::Buffered { meta, body }) => done(FetchOutcome::Ok {
            status: meta.status,
            status_text: meta.status_text,
            final_url: meta.final_url.to_string(),
            headers: flat_headers(&meta.headers),
            body: body.to_vec(),
        }),
        Ok(FetchResult::Stream { meta, peek_buf, shared }) => {
            // First thing, before anything can yield: the fetcher is already
            // pushing chunks, and nobody replays them.
            let chunks = shared.subscribe_with_cap(PUMP_QUEUE);
            let (producer, ring) = match gosub_ipc::ring::RingProducer::create(RING_CAPACITY) {
                Ok(pair) => pair,
                Err(e) => return done(FetchOutcome::Error(format!("could not set up a body stream: {e}"))),
            };
            Performed::Streaming {
                outcome: FetchOutcome::Streaming {
                    status: meta.status,
                    status_text: meta.status_text,
                    final_url: meta.final_url.to_string(),
                    headers: flat_headers(&meta.headers),
                    peek: peek_buf.as_ref().to_vec(),
                },
                ring,
                producer,
                chunks,
            }
        }
        Ok(FetchResult::Error(e)) => done(FetchOutcome::Error(e.to_string())),
        Err(_) => done(FetchOutcome::Error("the fetcher dropped the request".into())),
    }
}
