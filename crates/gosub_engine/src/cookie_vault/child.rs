//! The vault process: the cookie jars, and nothing else.
//!
//! The governing principle: no one process should hold both large secrets and
//! a large hostile-input surface. The broker deserializes untrusted frames from
//! every child, so the jars leave it and live here, in the least-authority
//! process of the model - the bare content baseline: no network, no files, no
//! devices; it moves bytes on the links it inherited and touches its own
//! memory. Compromised, it can answer the narrow questions below and nothing
//! more.
//!
//! Persistence is brokered: after every change the vault sends the broker a
//! snapshot of the zone's jar, and the broker writes it through the zone's
//! cookie store. That keeps the vault's filter at its tightest (it never opens
//! a file) at the cost of the broker seeing cookie state pass by - the
//! trade-off the PoC left open, decided this way because it works with any
//! embedder-supplied store and costs no capability.
//!
//! Two links: the broker's (control, embedder-API queries, snapshots out) and,
//! when the engine runs a network process, that process's own - so the cookie
//! *values* attached to requests flow network process ↔ vault directly and
//! never through the broker.

use crate::cookie_vault::protocol::{FromVault, ToVault};
use crate::engine::cookies::{CookieJar as _, DefaultCookieJar};
use gosub_ipc::{Endpoint, EndpointTx};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use url::Url;

/// Every zone's jar, keyed by the zone id the broker stamps.
type Jars = Arc<Mutex<HashMap<String, DefaultCookieJar>>>;

/// Entry point for the `vault` role. `net_link` is the network process's
/// direct line, when there is one.
pub fn serve(broker: Endpoint, net_link: Option<Endpoint>) -> i32 {
    gosub_sandbox::capture_process_title_region();
    gosub_sandbox::set_process_title("gosub-vault", "gosub: cookie vault");
    let jars: Jars = Arc::new(Mutex::new(HashMap::new()));
    let (broker_tx, mut broker_rx) = broker.split();
    let broker_tx = Arc::new(Mutex::new(broker_tx));

    // Threads before lockdown - and *running* before it: a thread's own
    // start-up makes syscalls (`rseq`, `set_robust_list`) the allowlist does
    // not carry, so the filter waits until the thread says it is past them.
    if let Some(link) = net_link {
        let jars = Arc::clone(&jars);
        let snapshots = Arc::clone(&broker_tx);
        let (started_tx, started_rx) = std::sync::mpsc::sync_channel::<()>(1);
        let spawned = std::thread::Builder::new().name("vault-net".into()).spawn(move || {
            let _ = started_tx.send(());
            serve_net(link, jars, snapshots)
        });
        if let Err(e) = spawned {
            eprintln!("[vault] could not start the network link: {e}");
            return 1;
        }
        if started_rx.recv_timeout(std::time::Duration::from_secs(5)).is_err() {
            eprintln!("[vault] the network link thread did not start");
            return 1;
        }
    }

    gosub_sandbox::lock_down_vault();

    // The broker link ends with the broker (recv error) or on Shutdown.
    while let Ok(msg) = broker_rx.recv::<ToVault>() {
        match msg {
            ToVault::Ping => {
                if broker_tx.lock().send(&FromVault::Pong).is_err() {
                    break;
                }
            }
            ToVault::Shutdown => break,
            msg => {
                if let Some(reply) = handle(msg, &jars, &broker_tx) {
                    if broker_tx.lock().send(&reply).is_err() {
                        break;
                    }
                }
            }
        }
    }
    0
}

/// The network process's line: `Get`/`Store` only. A `Store` still publishes
/// its snapshot on the broker link, which is where persistence happens.
fn serve_net(link: Endpoint, jars: Jars, snapshots: Arc<Mutex<EndpointTx>>) {
    let (tx, mut rx) = link.split();
    let tx = Arc::new(Mutex::new(tx));
    while let Ok(msg) = rx.recv::<ToVault>() {
        match msg {
            ToVault::Ping => {
                if tx.lock().send(&FromVault::Pong).is_err() {
                    return;
                }
            }
            msg @ (ToVault::Get { .. } | ToVault::Store { .. }) => {
                if let Some(reply) = handle(msg, &jars, &snapshots) {
                    if tx.lock().send(&reply).is_err() {
                        return;
                    }
                }
            }
            // Anything else is the broker's business; a network process asking
            // for it is confused or compromised, and gets nothing.
            other => eprintln!("[vault] refused {other:?} on the network link"),
        }
    }
}

/// One request against the jars. Replies go back on the asking link;
/// snapshots always go to the broker.
fn handle(msg: ToVault, jars: &Jars, snapshots: &Arc<Mutex<EndpointTx>>) -> Option<FromVault> {
    match msg {
        ToVault::OpenZone { zone, snapshot } => {
            jars.lock().insert(zone, snapshot.unwrap_or_default());
            None
        }
        ToVault::CloseZone { zone } => {
            jars.lock().remove(&zone);
            None
        }
        ToVault::Get {
            tag,
            scope,
            url,
            visible_only,
        } => {
            let header = Url::parse(&url).ok().and_then(|url| {
                let top = scope.top_level.as_deref().and_then(|t| Url::parse(t).ok());
                let jars = jars.lock();
                let jar = jars.get(&scope.zone)?;
                if visible_only {
                    // The document.cookie view: the same matching, over a jar
                    // with the HttpOnly cookies removed.
                    let mut visible = jar.clone();
                    for cookies in visible.entries.values_mut() {
                        cookies.retain(|c| !c.http_only);
                    }
                    visible.get_request_cookies(&url, top.as_ref(), scope.samesite.into())
                } else {
                    jar.get_request_cookies(&url, top.as_ref(), scope.samesite.into())
                }
            });
            Some(FromVault::Cookies { tag, header })
        }
        ToVault::Store {
            zone,
            url,
            top_level,
            set_cookie,
        } => {
            let Ok(url) = Url::parse(&url) else {
                return None;
            };
            let top = top_level.as_deref().and_then(|t| Url::parse(t).ok());
            let mut headers = http::HeaderMap::new();
            for value in set_cookie {
                if let Ok(value) = http::HeaderValue::from_str(&value) {
                    headers.append(http::header::SET_COOKIE, value);
                }
            }
            let snapshot = {
                let mut jars = jars.lock();
                let jar = jars.get_mut(&zone)?;
                jar.store_response_cookies(&url, &headers, top.as_ref());
                jar.clone()
            };
            let _ = snapshots.lock().send(&FromVault::Snapshot { zone, jar: snapshot });
            None
        }
        ToVault::GetAll { tag, zone } => {
            let cookies = jars
                .lock()
                .get(&zone)
                .map(|jar| {
                    jar.get_all_cookies()
                        .into_iter()
                        .map(|(url, cookie)| (url.to_string(), cookie))
                        .collect()
                })
                .unwrap_or_default();
            Some(FromVault::All { tag, cookies })
        }
        ToVault::Clear { zone } => mutate(jars, snapshots, &zone, |jar| jar.clear()),
        ToVault::Remove { zone, url, name } => mutate(jars, snapshots, &zone, |jar| {
            if let Ok(url) = Url::parse(&url) {
                jar.remove_cookie(&url, &name);
            }
        }),
        ToVault::RemoveForUrl { zone, url } => mutate(jars, snapshots, &zone, |jar| {
            if let Ok(url) = Url::parse(&url) {
                jar.remove_cookies_for_url(&url);
            }
        }),
        ToVault::PurgeExpired { zone } => mutate(jars, snapshots, &zone, |jar| jar.purge_expired()),
        ToVault::Ping | ToVault::Shutdown => None,
    }
}

/// Apply `change` to a zone's jar and publish the result.
fn mutate(
    jars: &Jars,
    snapshots: &Arc<Mutex<EndpointTx>>,
    zone: &str,
    change: impl FnOnce(&mut DefaultCookieJar),
) -> Option<FromVault> {
    let snapshot = {
        let mut jars = jars.lock();
        let jar = jars.get_mut(zone)?;
        change(jar);
        jar.clone()
    };
    let _ = snapshots.lock().send(&FromVault::Snapshot {
        zone: zone.to_string(),
        jar: snapshot,
    });
    None
}
