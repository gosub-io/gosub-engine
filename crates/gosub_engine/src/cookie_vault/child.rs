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

use crate::cookie_vault::protocol::{CookieScope, FromVault, Ticket, ToVault};
use crate::engine::cookies::{request_context, CookieJar as _, DefaultCookieJar, SameSiteContext};
use gosub_ipc::{Endpoint, EndpointTx};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use url::Url;

/// Every zone's jar, keyed by the zone id the broker stamps.
type Jars = Arc<Mutex<HashMap<String, DefaultCookieJar>>>;

/// Tickets the broker granted, each for one request of the network process.
type Grants = Arc<Mutex<HashMap<Ticket, (CookieScope, Instant, Used)>>>;

/// What a ticket was spent on: a request reads its cookies and stores its
/// response's once per redirect hop.
#[derive(Default)]
struct Used {
    gets: usize,
    stores: usize,
    /// The chain's context so far, `None` before the first `Get`: the grant's,
    /// made stricter by every hop (a chain is only as same-site as its least
    /// same-site hop).
    chain: Option<SameSiteContext>,
    /// Every URL the ticket was used at other than the granted one: hops only
    /// the network process saw, for the broker to check at revoke against the
    /// redirects it was told about.
    hops: Vec<String>,
}

/// Most `Get`s, and most `Store`s, one ticket buys. A chain at its longest is
/// 21 hops (20 redirects are followed), a retry runs it again and an
/// authentication challenge resends a hop; past this the network process is
/// claiming hops no request makes, and the rest go without cookies.
const MAX_CLAIMS_PER_TICKET: usize = 64;

/// Whether `hop` is the resource at `granted`: the same URL, the fragment
/// aside (it never goes on the wire), or its `http` to `https` upgrade, which
/// HSTS and mixed content make without a redirect.
fn same_resource(granted: &Url, hop: &Url) -> bool {
    let mut granted = granted.clone();
    let mut hop = hop.clone();
    granted.set_fragment(None);
    hop.set_fragment(None);
    if granted == hop {
        return true;
    }
    granted.scheme() == "http" && hop.scheme() == "https" && granted.set_scheme("https").is_ok() && granted == hop
}

/// Longer than any request may live; a grant the broker never revoked
/// (it died mid-request) goes away on its own.
const GRANT_TTL: Duration = Duration::from_secs(300);
/// More outstanding grants than this is a broker gone wrong, not load.
const MAX_GRANTS: usize = 4096;

/// Entry point for the `vault` role. `net_link` is the network process's
/// direct line, when there is one.
pub fn serve(broker: Endpoint, net_link: Option<Endpoint>) -> i32 {
    gosub_sandbox::capture_process_title_region();
    gosub_sandbox::set_process_title("gosub-vault", "gosub: cookie vault");
    let jars: Jars = Arc::new(Mutex::new(HashMap::new()));
    let grants: Grants = Arc::new(Mutex::new(HashMap::new()));
    let (broker_tx, mut broker_rx) = broker.split();
    let broker_tx = Arc::new(Mutex::new(broker_tx));

    // Threads before lockdown - and *running* before it: a thread's own
    // start-up makes syscalls (`rseq`, `set_robust_list`) the allowlist does
    // not carry, so the filter waits until the thread says it is past them.
    // No thread can start after it, so the one network-link thread serves
    // every line: the first, then each a respawned network process is given
    // (`ToVault::NetLine`), in turn.
    let mut next_lines = None;
    if let Some(link) = net_link {
        let jars = Arc::clone(&jars);
        let grants = Arc::clone(&grants);
        let snapshots = Arc::clone(&broker_tx);
        let (lines_tx, lines_rx) = std::sync::mpsc::channel::<Endpoint>();
        next_lines = Some(lines_tx);
        let (started_tx, started_rx) = std::sync::mpsc::sync_channel::<()>(1);
        let spawned = std::thread::Builder::new().name("vault-net".into()).spawn(move || {
            let _ = started_tx.send(());
            serve_net(link, Arc::clone(&jars), Arc::clone(&grants), Arc::clone(&snapshots));
            while let Ok(link) = lines_rx.recv() {
                serve_net(link, Arc::clone(&jars), Arc::clone(&grants), Arc::clone(&snapshots));
            }
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
            ToVault::Grant { tag, scope } => {
                let mut grants = grants.lock();
                grants.retain(|_, (_, since, _)| since.elapsed() < GRANT_TTL);
                let reply = if grants.len() >= MAX_GRANTS {
                    eprintln!("[vault] refusing a grant: {MAX_GRANTS} outstanding");
                    FromVault::Refused { tag }
                } else {
                    grants.insert(scope.ticket, (scope, Instant::now(), Used::default()));
                    FromVault::Granted { tag }
                };
                drop(grants);
                if broker_tx.lock().send(&reply).is_err() {
                    break;
                }
            }
            ToVault::Revoke { tag, ticket, redirects } => {
                let used = grants.lock().remove(&ticket);
                let unexplained = used.map_or_else(Vec::new, |(_, _, used)| unexplained_hops(used.hops, &redirects));
                if broker_tx.lock().send(&FromVault::Revoked { tag, unexplained }).is_err() {
                    break;
                }
            }
            ToVault::Audit { tag } => {
                let report = gosub_sandbox::audit::run(gosub_sandbox::audit::Role::Vault, &[]);
                if broker_tx.lock().send(&FromVault::Audit { tag, report }).is_err() {
                    break;
                }
            }
            // Read whether or not there is a thread to serve it: the
            // descriptors are on the link either way.
            ToVault::NetLine => match adopt_net_line(&mut broker_rx) {
                Ok(link) => match &next_lines {
                    Some(lines) => {
                        let _ = lines.send(link);
                    }
                    None => eprintln!("[vault] a network line for a vault started without one; dropped"),
                },
                Err(e) => {
                    eprintln!("[vault] the new network line did not arrive ({e})");
                    break;
                }
            },
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

/// The hops a ticket was used at that none of the `redirects` the broker was
/// told about explains.
fn unexplained_hops(hops: Vec<String>, redirects: &[String]) -> Vec<String> {
    let reported: Vec<Url> = redirects.iter().filter_map(|r| Url::parse(r).ok()).collect();
    hops.into_iter()
        .filter(|hop| Url::parse(hop).map_or(true, |hop| !reported.iter().any(|r| same_resource(r, &hop))))
        .collect()
}

/// A new network line: the descriptor follows its message twice, one per
/// half of the endpoint, since this process may not `dup`.
fn adopt_net_line(rx: &mut gosub_ipc::EndpointRx) -> std::io::Result<Endpoint> {
    let tx_fd = rx.recv_fd()?;
    let rx_fd = rx.recv_fd()?;
    Ok(Endpoint::from_halves(
        std::os::unix::net::UnixStream::from(tx_fd),
        std::os::unix::net::UnixStream::from(rx_fd),
    ))
}

/// The network process's line: `Get`/`Store` only, each under a granted
/// ticket, and acted on with the grant's scope - the zone and document the
/// broker recorded, whatever the message claims. A ticket buys a `Get` and a
/// `Store` per redirect hop, at web URLs, up to [`MAX_CLAIMS_PER_TICKET`].
///
/// A hop is the network process's word: it alone saw the `Location`. So a
/// `Get` anywhere but the granted URL is answered in the context the grant's
/// own document gives that URL, made stricter by the chain so far, never
/// laxer than the grant: a hop to another site gets `SameSite=None` cookies,
/// `Lax` too under a navigation, never `Strict` ones. What a false hop can
/// buy is what the page could have had the broker fetch for it anyway. Every
/// such URL is kept, and the broker checks them against the redirects it was
/// told about when it revokes the ticket.
///
/// A `Store` still publishes its snapshot on the broker link, which is where
/// persistence happens.
fn serve_net(link: Endpoint, jars: Jars, grants: Grants, snapshots: Arc<Mutex<EndpointTx>>) {
    let (tx, mut rx) = link.split();
    let tx = Arc::new(Mutex::new(tx));
    // Spends one of the ticket's `Get`s or `Store`s, and answers with the
    // scope to act on: the grant's, in the hop's context. A refused claim
    // spends nothing.
    let claim = |claimed: &CookieScope, url: &str, store: bool| -> Option<CookieScope> {
        let mut grants = grants.lock();
        let (scope, since, used) = grants.get_mut(&claimed.ticket)?;
        if since.elapsed() >= GRANT_TTL {
            return None;
        }
        let url = Url::parse(url)
            .ok()
            .filter(|u| matches!(u.scheme(), "http" | "https"))?;
        let spent = if store { &mut used.stores } else { &mut used.gets };
        if *spent >= MAX_CLAIMS_PER_TICKET {
            return None;
        }
        *spent += 1;
        let granted: SameSiteContext = scope.samesite.into();
        let at_granted = Url::parse(&scope.url).is_ok_and(|g| same_resource(&g, &url));
        let hop = if at_granted {
            granted
        } else {
            if !used.hops.iter().any(|h| h == url.as_str()) {
                used.hops.push(url.to_string());
            }
            // With no document recorded, the request is the document load,
            // and the granted URL is the document a hop is judged against: a
            // missing `top_level` must not make every hop same-site.
            let top = scope
                .top_level
                .as_deref()
                .and_then(|t| Url::parse(t).ok())
                .or_else(|| Url::parse(&scope.url).ok());
            request_context(top.as_ref(), &url, scope.navigation).stricter(granted)
        };
        // A store is answered in no context; only what goes out narrows.
        let chain = if store {
            used.chain.unwrap_or(granted)
        } else {
            let chain = used.chain.map_or(hop, |chain| chain.stricter(hop));
            used.chain = Some(chain);
            chain
        };
        Some(CookieScope {
            samesite: chain.into(),
            ..scope.clone()
        })
    };
    while let Ok(msg) = rx.recv::<ToVault>() {
        match msg {
            ToVault::Ping => {
                if tx.lock().send(&FromVault::Pong).is_err() {
                    return;
                }
            }
            ToVault::Get {
                tag,
                scope,
                url,
                visible_only,
            } => {
                let reply = match claim(&scope, &url, false) {
                    Some(scope) => handle(
                        ToVault::Get {
                            tag,
                            scope,
                            url,
                            visible_only,
                        },
                        &jars,
                        &snapshots,
                    ),
                    None => {
                        eprintln!("[vault] cookies asked for outside a grant; none given");
                        Some(FromVault::Cookies { tag, header: None })
                    }
                };
                if let Some(reply) = reply {
                    if tx.lock().send(&reply).is_err() {
                        return;
                    }
                }
            }
            ToVault::Store {
                tag,
                scope,
                url,
                set_cookie,
            } => {
                let stored = match claim(&scope, &url, true) {
                    Some(scope) => store(&jars, &snapshots, &scope, &url, set_cookie),
                    None => {
                        eprintln!("[vault] cookies stored outside a grant; refused");
                        false
                    }
                };
                // Answered either way: the asker is waiting, and keeps the
                // cookies on its reply unless they are `Stored`.
                let reply = if stored {
                    FromVault::Stored { tag }
                } else {
                    FromVault::Refused { tag }
                };
                if tx.lock().send(&reply).is_err() {
                    return;
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
            tag: _,
            scope,
            url,
            set_cookie,
        } => {
            store(jars, snapshots, &scope, &url, set_cookie);
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
        ToVault::Ping
        | ToVault::Shutdown
        | ToVault::Grant { .. }
        | ToVault::Revoke { .. }
        | ToVault::Audit { .. }
        | ToVault::NetLine => None,
    }
}

/// Record `set_cookie` from a response at `url` in the scope's zone. `false`
/// when nothing could be: the URL does not parse or the zone is not open.
fn store(
    jars: &Jars,
    snapshots: &Arc<Mutex<EndpointTx>>,
    scope: &CookieScope,
    url: &str,
    set_cookie: Vec<String>,
) -> bool {
    let Ok(url) = Url::parse(url) else {
        return false;
    };
    let top = scope.top_level.as_deref().and_then(|t| Url::parse(t).ok());
    let mut headers = http::HeaderMap::new();
    for value in set_cookie {
        if let Ok(value) = http::HeaderValue::from_bytes(value.as_bytes()) {
            headers.append(http::header::SET_COOKIE, value);
        }
    }
    let mut stored = false;
    mutate(jars, snapshots, &scope.zone, |jar| {
        jar.store_response_cookies(&url, &headers, top.as_ref());
        stored = true;
    });
    stored
}

/// Apply `change` to a zone's jar and publish the result. The snapshot goes
/// out under the jars lock: a `CloseZone` handled after the change is then
/// also after its snapshot on the broker link, which `close_zone` relies on.
fn mutate(
    jars: &Jars,
    snapshots: &Arc<Mutex<EndpointTx>>,
    zone: &str,
    change: impl FnOnce(&mut DefaultCookieJar),
) -> Option<FromVault> {
    let mut jars = jars.lock();
    let jar = jars.get_mut(zone)?;
    change(jar);
    // A jar past the link's frame cap cannot be snapshotted, and then nothing
    // of it is persisted from here on: said, since nothing else would say it.
    if let Err(e) = snapshots.lock().send(&FromVault::Snapshot {
        zone: zone.to_string(),
        jar: jar.clone(),
    }) {
        eprintln!("[vault] zone {zone}: jar snapshot not sent, its cookies are not being persisted: {e}");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::process::protocol::SameSite;

    /// A cookie the network process sends as UTF-8 is recorded as such, and
    /// `Stored` means the jar holds it; a zone that is not open stores nothing.
    #[test]
    fn a_non_ascii_cookie_is_stored() {
        let jars: Jars = Arc::new(Mutex::new(HashMap::new()));
        jars.lock().insert("z".into(), DefaultCookieJar::default());
        let (ours, _broker) = gosub_ipc::local_pair();
        let snapshots = Arc::new(Mutex::new(ours.split().0));
        let scope = CookieScope {
            ticket: 0,
            url: "https://site.test/".into(),
            zone: "z".into(),
            top_level: None,
            samesite: SameSite::SameSite,
            navigation: false,
        };
        let set_cookie = vec!["name=h\u{e9}llo; Path=/".to_string()];
        assert!(store(
            &jars,
            &snapshots,
            &scope,
            "https://site.test/",
            set_cookie.clone()
        ));
        let url = Url::parse("https://site.test/").unwrap();
        let header = jars.lock()["z"].get_request_cookies(&url, None, SameSite::SameSite.into());
        assert_eq!(header.as_deref(), Some("name=h\u{e9}llo"));

        let closed = CookieScope {
            zone: "closed".into(),
            ..scope
        };
        assert!(!store(&jars, &snapshots, &closed, "https://site.test/", set_cookie));
    }

    /// The audit lets through a hop a reported redirect explains, its `https`
    /// upgrade and a fragment, and names the rest.
    #[test]
    fn the_audit_names_only_hops_no_redirect_explains() {
        let redirects = vec!["http://site.test/next".to_string()];
        let hops = vec![
            "http://site.test/next".to_string(),
            "https://site.test/next".to_string(),
            "http://site.test/next#part".to_string(),
            "https://elsewhere.test/".to_string(),
            "not a url".to_string(),
        ];
        assert_eq!(
            unexplained_hops(hops, &redirects),
            vec!["https://elsewhere.test/".to_string(), "not a url".to_string()]
        );
    }

    /// An upgrade goes one way: an `https` grant does not cover its `http` form.
    #[test]
    fn same_resource_covers_the_upgrade_only() {
        let url = |s: &str| Url::parse(s).unwrap();
        assert!(same_resource(&url("http://a.test/x"), &url("https://a.test/x")));
        assert!(!same_resource(&url("https://a.test/x"), &url("http://a.test/x")));
        assert!(!same_resource(&url("http://a.test/x"), &url("https://a.test/y")));
    }
}
