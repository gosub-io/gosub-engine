//! The network process: the only part of the engine that may open a socket.
//!
//! What only Linux can do - pass a ring fd for a streamed body - lives in
//! `platform`; the same API elsewhere declines, so this file has no platform
//! branches of its own.

use crate::net::fetcher::{Fetcher, FetcherConfig};
use crate::net::process::protocol::{FetchOutcome, FromNet, NetFetch, RequestTag, ToNet};
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

use platform::Streamed;

/// How long a shutdown drain waits for in-flight requests before giving up.
/// Shorter than the broker's `SHUTDOWN_GRACE`, so a draining child exits on
/// its own rather than being killed mid-drain.
const DRAIN_GRACE: std::time::Duration = std::time::Duration::from_secs(3);

/// Run as the network process until the broker disconnects or says to stop.
pub fn serve(link: Endpoint) -> i32 {
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
                let handle = runtime.spawn(async move {
                    let performed = perform(&fetcher, fetch, token).await;
                    cancels.lock().remove(&tag);
                    // A write error means the broker went away; the recv loop
                    // notices the same and ends the process.
                    match performed {
                        Performed::Done(outcome) => {
                            let _ = link_tx.lock().send(&FromNet::Reply { tag, outcome });
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

/// What `perform` produced: a reply that travels whole, or a response head
/// whose body is still arriving and will follow it (see [`Streamed`]).
enum Performed {
    Done(FetchOutcome),
    Streaming(Streamed),
}

fn flat_headers(headers: &http::HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .filter_map(|(n, v)| v.to_str().ok().map(|v| (n.as_str().to_string(), v.to_string())))
        .collect()
}

/// Perform one request and flatten the result to something that can travel.
async fn perform(fetcher: &Arc<Fetcher>, fetch: NetFetch, cancel: CancellationToken) -> Performed {
    let streaming = fetch.streaming && platform::STREAMING;
    let done = Performed::Done;
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
    match result {
        Ok(FetchResult::Buffered { meta, body }) => done(FetchOutcome::Ok {
            status: meta.status,
            status_text: meta.status_text,
            final_url: meta.final_url.to_string(),
            headers: flat_headers(&meta.headers),
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
                headers: flat_headers(&meta.headers),
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
