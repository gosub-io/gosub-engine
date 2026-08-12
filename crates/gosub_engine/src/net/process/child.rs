//! The network process: the only part of the engine that may open a socket.

use crate::net::fetcher::{Fetcher, FetcherConfig};
use crate::net::process::protocol::{FetchOutcome, FromNet, NetFetch, ToNet};
use crate::net::types::{FetchHandle, FetchRequest, FetchResult};
use gosub_ipc::Endpoint;
use gosub_sonar::net::fetcher_context::NullContext;
use http::Method;
use std::str::FromStr;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use url::Url;

/// Run as the network process until the broker disconnects or says to stop.
pub fn serve(mut link: Endpoint) -> i32 {
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
    let fetcher = match Fetcher::new(FetcherConfig::default(), Arc::new(NullContext)) {
        Ok(f) => Arc::new(f),
        Err(e) => {
            eprintln!("[net] could not build the fetcher: {e}");
            return 1;
        }
    };

    let shutdown = CancellationToken::new();
    let fetcher_run = fetcher.clone();
    let cancel = shutdown.clone();
    runtime.spawn(async move { fetcher_run.run(cancel).await });

    // A read error ends the loop: it means the broker went away, which is a
    // normal end - the network process exists only to serve it.
    while let Ok(msg) = link.recv::<ToNet>() {
        match msg {
            ToNet::Ping => {
                if link.send(&FromNet::Pong).is_err() {
                    break;
                }
            }
            ToNet::Shutdown => break,
            ToNet::Fetch(fetch) => {
                let tag = fetch.tag;
                let outcome = runtime.block_on(perform(&fetcher, fetch));
                if link.send(&FromNet::Reply { tag, outcome }).is_err() {
                    break;
                }
            }
        }
    }

    shutdown.cancel();
    0
}

/// Perform one request and flatten the result to something that can travel.
async fn perform(fetcher: &Arc<Fetcher>, fetch: NetFetch) -> FetchOutcome {
    let url = match Url::parse(&fetch.url) {
        Ok(u) => u,
        Err(e) => return FetchOutcome::Error(format!("bad url {}: {e}", fetch.url)),
    };
    let method = match Method::from_str(&fetch.method) {
        Ok(m) => m,
        Err(e) => return FetchOutcome::Error(format!("bad method {}: {e}", fetch.method)),
    };

    let mut headers = http::HeaderMap::new();
    for (name, value) in &fetch.headers {
        let parsed = http::header::HeaderName::from_str(name).ok().zip(value.parse().ok());
        if let Some((name, value)) = parsed {
            headers.insert(name, value);
        }
    }

    let req = FetchRequest::builder(method, url)
        .with_headers(headers)
        // Buffered: a streamed body cannot cross the link (see `protocol`).
        .with_streaming(false)
        .with_auto_decode(true)
        .build();

    let handle = FetchHandle {
        req_id: req.req_id,
        key: req.key_data.clone(),
        cancel: CancellationToken::new(),
    };

    let (tx, rx) = tokio::sync::oneshot::channel::<FetchResult>();
    fetcher.submit(req, handle, tx).await;

    match rx.await {
        Ok(FetchResult::Buffered { meta, body }) => FetchOutcome::Ok {
            status: meta.status,
            status_text: meta.status_text,
            final_url: meta.final_url.to_string(),
            headers: meta
                .headers
                .iter()
                .filter_map(|(n, v)| v.to_str().ok().map(|v| (n.as_str().to_string(), v.to_string())))
                .collect(),
            body: body.to_vec(),
        },
        Ok(FetchResult::Stream { meta, .. }) => {
            FetchOutcome::Error(format!("unexpected streamed body for {}", meta.final_url))
        }
        Ok(FetchResult::Error(e)) => FetchOutcome::Error(e.to_string()),
        Err(_) => FetchOutcome::Error("the fetcher dropped the request".into()),
    }
}
