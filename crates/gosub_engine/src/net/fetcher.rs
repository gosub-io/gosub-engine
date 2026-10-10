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
