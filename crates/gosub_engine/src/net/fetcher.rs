pub use gosub_sonar::net::fetcher::{Fetcher, FetcherConfig};
pub use gosub_sonar::net::fetcher_context::FetcherContext;

/// Build a [`FetcherConfig`] from the engine's settings store.
///
/// Deliberately an engine-side free function rather than a `FetcherConfig::from_config` method on
/// the gosub-sonar type: that would force sonar to know engine-specific setting keys. Any knob not
/// present falls back to [`FetcherConfig::default`] (the gosub-sonar defaults, including the user
/// agent).
pub fn fetcher_config_from(cfg: &gosub_config::Config) -> FetcherConfig {
    use std::time::Duration;

    // A body timeout of 0 means "no limit".
    let body_secs = cfg.get_uint("net.timeout.body_secs");
    // An empty net.user_agent (the default) means the computed compat UA.
    let user_agent = cfg.get_string("net.user_agent");
    let user_agent = if user_agent.is_empty() {
        default_user_agent(None)
    } else {
        user_agent
    };
    FetcherConfig {
        global_slots: cfg.get_uint("net.http.global_slots"),
        user_agent: Some(user_agent),
        h1_per_origin: cfg.get_uint("net.http.per_origin_h1"),
        h2_per_origin: cfg.get_uint("net.http.per_origin_h2"),
        connect_timeout: Duration::from_secs(cfg.get_uint("net.timeout.connect_secs") as u64),
        req_timeout: Duration::from_secs(cfg.get_uint("net.timeout.request_secs") as u64),
        read_idle_timeout: Duration::from_secs(cfg.get_uint("net.timeout.read_idle_secs") as u64),
        total_body_timeout: (body_secs > 0).then(|| Duration::from_secs(body_secs as u64)),
        // Resolution has to go through a `DnsResolver` to be visible: the HTTP client's own
        // lookup happens below sonar's level and emits no event, so `net.dns` stays silent
        // without one. `SystemResolver` is `getaddrinfo` with no policy attached - the same
        // resolution the client would do by itself - so this buys the timing and changes
        // nothing else.
        //
        // It applies no SSRF or DNS-rebinding protection. Neither does the default it
        // replaces, so this is not a regression, but a resolver that classifies addresses
        // is what should eventually sit here. See `gosub_sonar::net::dns`.
        dns_resolver: Some(std::sync::Arc::new(gosub_sonar::net::dns::SystemResolver)),
        ..FetcherConfig::default()
    }
}

/// Platform parenthetical for the User-Agent, matching what mainstream browsers
/// report on each OS (macOS is frozen at 10_15_7 industry-wide, Windows at NT 10.0).
fn ua_platform() -> &'static str {
    if cfg!(target_os = "windows") {
        "Windows NT 10.0; Win64; x64"
    } else if cfg!(target_os = "macos") {
        "Macintosh; Intel Mac OS X 10_15_7"
    } else if cfg!(target_arch = "aarch64") {
        "X11; Linux aarch64"
    } else {
        "X11; Linux x86_64"
    }
}

/// Compat-shaped `User-Agent` for this engine build:
///
/// `Mozilla/5.0 (<platform>) AppleWebKit/537.36 (KHTML, like Gecko) Gosub/<version>`
///
/// The `Mozilla/5.0` prefix, platform parenthetical and frozen WebKit token are the
/// lies every browser tells so sniffers serve modern markup instead of the legacy
/// path; `Gosub/<version>` is the honest engine identity. `product` appends the
/// embedder's token (e.g. `"Beacon/0.1.0"`) in the position sniffers expect a
/// browser name. Used whenever the `net.user_agent` setting is empty.
pub fn default_user_agent(product: Option<&str>) -> String {
    let base = format!(
        "Mozilla/5.0 ({}) AppleWebKit/537.36 (KHTML, like Gecko) Gosub/{}",
        ua_platform(),
        env!("CARGO_PKG_VERSION"),
    );
    match product {
        Some(product) => format!("{base} {product}"),
        None => base,
    }
}

use crate::engine::types::{EventChannel, ResourceChannel};
use crate::net::emitter::engine_event_emitter::EngineEventEmitter;
use crate::net::emitter::null_emitter::NullEmitter;
use crate::net::req_ref_tracker::{RequestRefTracker, RequestReferenceMap, REF_REGISTRY};
use crate::net::ssrf::AddressSpace;
use crate::net::types::{Initiator as EngineInitiator, ResourceKind as EngineResourceKind};
use gosub_sonar::net::observer::NetObserver;
use gosub_sonar::net::types::{Initiator, ResourceKind};
use gosub_sonar::types::RequestId;
use parking_lot::RwLock;
use std::sync::Arc;

/// Engine-side implementation of FetcherContext.
/// Bridges the net-layer Fetcher to engine events and tab tracking.
///
/// The fetcher hands us the opaque sonar-side reference tags; we resolve them back to the
/// engine's rich [`RequestReference`](crate::net::req_ref_tracker::RequestReference) via
/// [`REF_REGISTRY`] before touching engine state.
pub struct EngineNetContext {
    /// Per-resource events go to the resource stream; throttled navigation and download
    /// progress goes to the control bus.
    pub resource_tx: ResourceChannel,
    pub event_tx: EventChannel,
    pub request_reference_map: Arc<RwLock<RequestReferenceMap>>,
    pub request_ref_tracker: Arc<RequestRefTracker>,
    /// Each tab's jar and document: what the cookie hooks answer from, hop by
    /// hop, for the tab a request's reference belongs to.
    pub tab_identities: Arc<crate::net::tab_identity::TabIdentityRegistry>,
    /// The address space of the documents this fetcher serves subresources
    /// for: anything more private is refused at every hop (see
    /// [`crate::net::ssrf`]). Hostnames are refused by the strict resolver;
    /// this covers the IP literals it never sees. `Loopback`, which also serves
    /// navigations, refuses nothing.
    pub reach: AddressSpace,
}

/// The configuration of a fetcher that serves documents in `reach`: `cfg`
/// with the strict resolver, which fails closed on any answer beyond `reach`
/// and, being the only resolver the client has, cannot be rebound around.
/// Nothing is beyond `Loopback`, so that one is `cfg` as it is.
pub fn reach_config(cfg: &FetcherConfig, reach: AddressSpace) -> FetcherConfig {
    if reach == AddressSpace::Loopback {
        return cfg.clone();
    }
    FetcherConfig {
        dns_resolver: Some(Arc::new(crate::net::ssrf::StrictResolver { reach })),
        ..cfg.clone()
    }
}

/// The observer that reports a request to the embedder: the engine's emitter
/// for the tab the request's reference belongs to, wrapped for timing where
/// that is compiled in. Nothing (a null observer) for a reference no tab
/// owns. Shared by the in-process fetcher, which asks per request, and the
/// network-process dispatch, which has no fetcher to ask it.
pub(crate) fn observer_for_request(
    resource_tx: &ResourceChannel,
    event_tx: &EventChannel,
    request_reference_map: &Arc<RwLock<RequestReferenceMap>>,
    reference: gosub_sonar::RequestReference,
    req_id: RequestId,
    kind: ResourceKind,
    initiator: Initiator,
) -> Arc<dyn NetObserver + Send + Sync> {
    let Some(reference) = REF_REGISTRY.from_net(reference) else {
        log::trace!("Cannot resolve net reference {:?} to an engine reference", reference);
        return Arc::new(NullEmitter) as Arc<dyn NetObserver + Send + Sync>;
    };

    // Recover the rich (kind, initiator) pair registered when the request was built;
    // sonar only carries its own coarse classification through the pipeline.
    let (kind, initiator) = REF_REGISTRY
        .request_meta(req_id)
        .unwrap_or_else(|| (EngineResourceKind::from_net(kind), EngineInitiator::from_net(initiator)));

    let tab_id = request_reference_map.read().get(&reference).copied();
    let Some(tab_id) = tab_id else {
        log::trace!("Cannot find the request reference for reference {:?}", reference);
        return Arc::new(NullEmitter) as Arc<dyn NetObserver + Send + Sync>;
    };
    let observer = Arc::new(EngineEventEmitter::new(
        tab_id,
        req_id,
        reference,
        resource_tx.clone(),
        event_tx.clone(),
        kind,
        initiator,
    )) as Arc<dyn NetObserver + Send + Sync>;

    // Timing decorates the emitter rather than replacing it: it reads the
    // fetch timings off each event in passing and forwards the event on.
    // With the feature off no wrapper is built and sonar emits into exactly
    // what it does today.
    #[cfg(feature = "timing")]
    let observer = {
        // Only the main document's request is referenced by its navigation;
        // sub-resources reference a Document, which carries no navigation, so
        // they record unattributed rather than against a guessed one.
        let scope = match reference {
            crate::net::req_ref_tracker::RequestReference::Navigation(nav_id) => {
                Some(gosub_shared::timing::ScopeId(nav_id.0))
            }
            _ => None,
        };
        Arc::new(crate::net::emitter::timing_emitter::TimingEmitter::wrap(
            observer, kind, scope,
        )) as Arc<dyn NetObserver + Send + Sync>
    };

    observer
}

impl EngineNetContext {
    /// The identity of the tab a request's reference belongs to, and the
    /// engine reference itself. A navigation's reference is shared by its own
    /// request and the loads of the document it produces, so it alone does not
    /// say which a request is (see `TabIdentity::cookie_context`).
    fn identity_for(
        &self,
        reference: gosub_sonar::RequestReference,
    ) -> Option<(
        crate::net::tab_identity::TabIdentity,
        crate::net::req_ref_tracker::RequestReference,
    )> {
        let reference = REF_REGISTRY.from_net(reference)?;
        let tab_id = self.request_reference_map.read().get(&reference).copied()?;
        let identity = self.tab_identities.get(tab_id)?;
        Some((identity, reference))
    }
}

/// Run `f`, which may block on a jar that answers over IPC (the vault), from
/// inside the fetcher's async redirect loop: on a multi-threaded runtime the
/// worker hands its other tasks off first.
fn blocking_jar_call<T>(f: impl FnOnce() -> T) -> T {
    match tokio::runtime::Handle::try_current().map(|h| h.runtime_flavor()) {
        Ok(tokio::runtime::RuntimeFlavor::MultiThread) => tokio::task::block_in_place(f),
        _ => f(),
    }
}

impl FetcherContext for EngineNetContext {
    fn observer_for(
        &self,
        reference: gosub_sonar::RequestReference,
        req_id: RequestId,
        kind: ResourceKind,
        initiator: Initiator,
    ) -> Arc<dyn NetObserver + Send + Sync> {
        observer_for_request(
            &self.resource_tx,
            &self.event_tx,
            &self.request_reference_map,
            reference,
            req_id,
            kind,
            initiator,
        )
    }

    fn is_url_allowed(&self, url: &url::Url) -> bool {
        if self.reach == AddressSpace::Loopback {
            return true;
        }
        match crate::net::ssrf::literal_verdict(url, self.reach) {
            Some(reason) => {
                log::info!("blocked {url}: {reason}");
                false
            }
            None => true,
        }
    }

    // Asked at every hop, so a cookie a redirect sets rides on the next one.
    // A navigation's hops are judged against who started it, a document's
    // loads against the document; either over the whole chain so far, so a
    // chain that leaves the site stays cross-site when it comes back, and by
    // the hop's method, so a cross-site `POST` gets no `Lax` cookies.
    fn cookies_for_hop(
        &self,
        reference: gosub_sonar::RequestReference,
        hop: &gosub_sonar::CookieHop<'_>,
    ) -> Option<String> {
        let (identity, reference) = self.identity_for(reference)?;
        // The page a load was made for, not the tab's newest top level: a
        // page that is being navigated away from is still loading.
        let top_level = identity.cookie_document(Some(reference), None);
        let context = identity.cookie_context(Some(reference), None, hop.url, hop.url_list, hop.method);
        blocking_jar_call(|| {
            identity
                .cookie_jar
                .read()
                .get_request_cookies(hop.url, top_level, context)
        })
    }

    // Two requests get the same answers from the hooks when they ask the same
    // jar in the same context: a navigation from the same initiator, or a load
    // of the same document. Only then may they share a fetch.
    fn cookie_jar_key(&self, reference: gosub_sonar::RequestReference) -> String {
        match self.identity_for(reference) {
            Some((identity, reference)) => format!(
                "{:x} {} {}",
                identity.cookie_jar.jar_id(),
                identity
                    .cookie_document(Some(reference), None)
                    .map_or("", |u| u.as_str()),
                identity.cookie_context_key(Some(reference))
            ),
            None => String::new(),
        }
    }

    fn on_cookies_received(&self, reference: gosub_sonar::RequestReference, url: &url::Url, values: &[&str]) {
        let Some((identity, reference)) = self.identity_for(reference) else {
            return;
        };
        let mut headers = http::HeaderMap::new();
        for value in values {
            if let Ok(value) = http::HeaderValue::from_bytes(value.as_bytes()) {
                headers.append(http::header::SET_COOKIE, value);
            }
        }
        blocking_jar_call(|| {
            identity.cookie_jar.write().store_response_cookies(
                url,
                &headers,
                identity.cookie_document(Some(reference), None),
            )
        });
    }

    fn on_ref_active(&self, reference: gosub_sonar::RequestReference) {
        if let Some(reference) = REF_REGISTRY.from_net(reference) {
            self.request_ref_tracker.inc(&reference);
        }
    }

    fn on_ref_done(&self, reference: gosub_sonar::RequestReference) {
        if let Some(reference) = REF_REGISTRY.from_net(reference) {
            self.request_ref_tracker
                .dec_and_maybe_cleanup(&reference, &self.request_reference_map);
        }
    }
}

#[cfg(test)]
mod dns_resolver_tests {
    use super::*;
    use crate::engine::settings_store::default_config;

    /// `net.dns` timings only exist when resolution goes through a `DnsResolver`; the HTTP
    /// client's built-in lookup is below sonar's level and emits nothing. Dropping it from
    /// the config would silence that namespace without breaking anything else, which is a
    /// hard failure to notice - hence this test.
    #[test]
    fn a_resolver_is_installed_so_dns_timings_exist() {
        let cfg = fetcher_config_from(&default_config());
        assert!(
            cfg.dns_resolver.is_some(),
            "no DnsResolver configured - net.dns will be silent in the running engine"
        );
    }

    /// The installed resolver has to actually resolve. It hands sonar `host:0` and lets the
    /// fetcher substitute the scheme's default port, so a mistake there yields zero
    /// addresses and every connection fails - worth pinning rather than assuming.
    #[tokio::test(flavor = "current_thread")]
    async fn the_installed_resolver_resolves_localhost() {
        let cfg = fetcher_config_from(&default_config());
        let resolver = cfg.dns_resolver.expect("resolver installed");

        let addrs = resolver.resolve("localhost").await.expect("localhost resolves");
        assert!(!addrs.is_empty(), "resolver returned no addresses for localhost");
    }
}

/// Cookies across real redirect hops, through a fetcher and the engine's hook: each hop is
/// judged by its method and the chain before it.
#[cfg(test)]
mod redirect_cookie_tests {
    use super::*;
    use crate::engine::cookies::{CookieJar, CookieJarHandle, DefaultCookieJar};
    use crate::engine::types::NavigationId;
    use crate::net::req_ref_tracker::RequestReference;
    use crate::tab::TabId;
    use std::collections::HashMap;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use url::Url;

    type Seen = Arc<parking_lot::Mutex<HashMap<String, String>>>;

    /// A listener on `ip`, its port, and what it saw: the first request for each path.
    async fn listen(ip: &str) -> (tokio::net::TcpListener, u16, Seen) {
        let listener = tokio::net::TcpListener::bind((ip, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        (listener, port, Seen::default())
    }

    /// Answer every request with `route(path)`: a status line and headers.
    fn serve(listener: tokio::net::TcpListener, seen: Seen, route: impl Fn(&str) -> String + Send + 'static) {
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut buf = vec![0u8; 8192];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let request =
                    cow_utils::CowUtils::cow_to_ascii_lowercase(&*String::from_utf8_lossy(&buf[..n])).into_owned();
                let path = request.split(' ').nth(1).unwrap_or("").to_string();
                let head = route(&path);
                seen.lock().entry(path).or_insert(request);
                let response = format!("{head}Content-Length: 0\r\nConnection: close\r\n\r\n");
                let _ = stream.write_all(response.as_bytes()).await;
            }
        });
    }

    /// Every name is the loopback address: `b.test` is a second site on the same machine, as
    /// `127.0.0.2` would be on Linux, where all of 127/8 is loopback, but not on macOS.
    struct Loopback;

    impl gosub_sonar::DnsResolver for Loopback {
        fn resolve(&self, _host: &str) -> gosub_sonar::Resolving {
            Box::pin(async { Ok(vec![std::net::SocketAddr::from(([127, 0, 0, 1], 0))]) })
        }
    }

    /// A fetcher whose navigation `reference` belongs to a tab showing `top_level`, with a jar
    /// holding `cookies` for `site`. The navigation is the user's own, started over the
    /// network, so its hops are judged from its first URL.
    fn navigating_tab(top_level: &Url, site: &Url, cookies: &[&str]) -> (Arc<Fetcher>, gosub_sonar::RequestReference) {
        let mut jar = DefaultCookieJar::new();
        let mut headers = http::HeaderMap::new();
        for cookie in cookies {
            headers.append(http::header::SET_COOKIE, cookie.parse().unwrap());
        }
        jar.store_response_cookies(site, &headers, None);
        let tab = TabId::new();
        let identities = Arc::new(crate::net::tab_identity::TabIdentityRegistry::new());
        let jar: Box<dyn CookieJar + Send + Sync> = Box::new(jar);
        identities.register(tab, CookieJarHandle::from(jar));
        identities.set_top_level(tab, top_level.clone());
        let navigation = NavigationId::new();
        identities.start_navigation(tab, navigation, None);
        let reference = RequestReference::Navigation(navigation);
        let map = Arc::new(RwLock::new(RequestReferenceMap::new()));
        map.write().insert(reference, tab);
        let context = EngineNetContext {
            resource_tx: tokio::sync::broadcast::channel(16).0,
            event_tx: tokio::sync::broadcast::channel(16).0,
            request_reference_map: map,
            request_ref_tracker: Arc::new(RequestRefTracker::new()),
            tab_identities: identities,
            reach: AddressSpace::Loopback,
        };
        let config = FetcherConfig {
            proxy: gosub_sonar::ProxyConfig::Disabled,
            dns_resolver: Some(Arc::new(Loopback)),
            ..FetcherConfig::default()
        };
        let fetcher = Arc::new(Fetcher::new(config, Arc::new(context)).unwrap());
        tokio::spawn({
            let fetcher = fetcher.clone();
            async move { fetcher.run(tokio_util::sync::CancellationToken::new()).await }
        });
        (fetcher, REF_REGISTRY.to_net(reference))
    }

    fn navigation(
        method: http::Method,
        url: &Url,
        reference: gosub_sonar::RequestReference,
    ) -> crate::net::types::FetchRequest {
        let mut req = crate::net::types::FetchRequest::builder(method.clone(), url.clone())
            .with_reference(reference)
            .with_kind(ResourceKind::Primary);
        if method == http::Method::POST {
            req = req.with_body(crate::net::types::RequestBody::form("a=1"));
        }
        req.build()
    }

    fn cookies_at(seen: &Seen, path: &str) -> String {
        let request = seen
            .lock()
            .get(path)
            .cloned()
            .unwrap_or_else(|| panic!("{path} never requested"));
        request
            .lines()
            .find_map(|line| line.strip_prefix("cookie: ").map(str::to_string))
            .unwrap_or_default()
    }

    /// A 307 keeps a cross-site navigation's POST, which is not a safe method: no `Lax`
    /// cookies on that hop. A 302 turns it into a GET, which gets them.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_cross_site_post_hop_carries_no_lax_cookies() {
        let (b_listener, b, b_seen) = listen("127.0.0.1").await;
        serve(b_listener, b_seen.clone(), |_| "HTTP/1.1 200 OK\r\n".into());
        let (a_listener, a, a_seen) = listen("127.0.0.1").await;
        serve(a_listener, a_seen, move |path| {
            let status = if path == "/307" {
                "307 Temporary Redirect"
            } else {
                "302 Found"
            };
            format!("HTTP/1.1 {status}\r\nLocation: http://b.test:{b}/from{path}\r\n")
        });

        let site_b = Url::parse(&format!("http://b.test:{b}/")).unwrap();
        let top = Url::parse(&format!("http://127.0.0.1:{a}/307")).unwrap();
        let (fetcher, reference) = navigating_tab(&top, &site_b, &["lax=1; SameSite=Lax; Path=/"]);
        for path in ["307", "302"] {
            let url = Url::parse(&format!("http://127.0.0.1:{a}/{path}")).unwrap();
            fetcher.fetch(navigation(http::Method::POST, &url, reference)).await;
        }

        assert_eq!(
            cookies_at(&b_seen, "/from/307"),
            "",
            "a POST kept across sites carries no Lax cookie"
        );
        assert_eq!(
            cookies_at(&b_seen, "/from/302"),
            "lax=1",
            "the GET after a 302 carries it"
        );
    }

    /// A navigation that leaves the site and comes back is cross-site on its way back:
    /// `Strict` cookies stay home, `Lax` ones still ride on the (safe) navigation.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_chain_through_another_site_stays_cross_site() {
        let (a_listener, a, a_seen) = listen("127.0.0.1").await;
        let (b_listener, b, b_seen) = listen("127.0.0.1").await;
        serve(b_listener, b_seen, move |_| {
            format!("HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{a}/back\r\n")
        });
        serve(a_listener, a_seen.clone(), move |path| match path {
            "/away" => format!("HTTP/1.1 302 Found\r\nLocation: http://b.test:{b}/bounce\r\n"),
            _ => "HTTP/1.1 200 OK\r\n".into(),
        });

        let site_a = Url::parse(&format!("http://127.0.0.1:{a}/")).unwrap();
        let top = Url::parse(&format!("http://127.0.0.1:{a}/away")).unwrap();
        let cookies = ["strict=1; SameSite=Strict; Path=/", "lax=1; SameSite=Lax; Path=/"];
        let (fetcher, reference) = navigating_tab(&top, &site_a, &cookies);
        let home = Url::parse(&format!("http://127.0.0.1:{a}/home")).unwrap();
        fetcher.fetch(navigation(http::Method::GET, &home, reference)).await;
        fetcher.fetch(navigation(http::Method::GET, &top, reference)).await;

        assert_eq!(
            cookies_at(&a_seen, "/home"),
            "strict=1; lax=1",
            "a same-site hop carries both"
        );
        assert_eq!(
            cookies_at(&a_seen, "/back"),
            "lax=1",
            "back from another site, Strict stays home"
        );
    }
}
