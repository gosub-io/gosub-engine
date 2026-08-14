// Re-export the net-layer Fetcher and supporting types from the external gosub-sonar crate
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

use crate::engine::types::EventChannel;
use crate::net::emitter::engine_event_emitter::EngineEventEmitter;
use crate::net::emitter::null_emitter::NullEmitter;
use crate::net::req_ref_tracker::{RequestRefTracker, RequestReferenceMap, REF_REGISTRY};
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
    pub event_tx: EventChannel,
    pub request_reference_map: Arc<RwLock<RequestReferenceMap>>,
    pub request_ref_tracker: Arc<RequestRefTracker>,
}

impl FetcherContext for EngineNetContext {
    fn observer_for(
        &self,
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

        let guard = self.request_reference_map.read();
        match guard.get(&reference) {
            Some(&tab_id) => Arc::new(EngineEventEmitter::new(
                tab_id,
                req_id,
                reference,
                self.event_tx.clone(),
                kind,
                initiator,
            )),
            None => {
                log::trace!("Cannot find the request reference for reference {:?}", reference);
                Arc::new(NullEmitter) as Arc<dyn NetObserver + Send + Sync>
            }
        }
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
