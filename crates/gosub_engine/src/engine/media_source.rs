//! Answers the media store's requests for bytes, through the zone's fetcher.
//!
//! The store does not fetch and has no way to: it names what it needs and waits for the
//! bytes to appear in the resource handoff. This is what puts them there -- on the same path
//! as everything else the page loads, which means the policy applies, the cache applies, and
//! the request shows up in the network panel. It replaces a blocking fetch the store used to
//! make with its own HTTP client, outside all three.

use crate::engine::types::{IoChannel, RequestId};
use crate::net::req_ref_tracker::{RequestReference, REF_REGISTRY};
use crate::net::submit_to_io;
use crate::net::types::{FetchRequest, FetchResult, Initiator, Priority, ResourceKind};
use crate::zone::ZoneId;
use gosub_render_pipeline::common::media::MediaSource;
use http::Method;
use parking_lot::RwLock;
use tokio::runtime::Handle;
use url::Url;

/// A media source bound to one tab: its zone's fetcher, and whatever document it is showing.
pub struct EngineMediaSource {
    zone_id: ZoneId,
    io_tx: IoChannel,
    runtime: Handle,
    /// The document the request is for, and the navigation it belongs to. The URL decides
    /// the `Referer` and whether a `file://` URL may be loaded at all; the reference is what
    /// makes the request *visible* -- without one the fetcher attaches a null observer, and
    /// the request happens with nothing to show for it in the network panel. `None` before
    /// the first navigation commits.
    document: RwLock<Option<(Url, RequestReference)>>,
    /// `Accept-Language` sent with the request, matching the rest of the page's fetches.
    accept_language: Option<String>,
}

impl EngineMediaSource {
    pub fn new(zone_id: ZoneId, io_tx: IoChannel, runtime: Handle, accept_language: Option<String>) -> Self {
        Self {
            zone_id,
            io_tx,
            runtime,
            document: RwLock::new(None),
            accept_language,
        }
    }

    /// Tell the source which document its requests belong to, and which navigation to file
    /// them under. Called when a navigation commits, before the document is handed to the
    /// renderer.
    pub fn set_document(&self, url: Option<Url>, reference: RequestReference) {
        *self.document.write() = url.map(|url| (url, reference));
    }
}

impl MediaSource for EngineMediaSource {
    fn scope(&self) -> gosub_shared::subresource::Scope {
        self.zone_id.as_scope()
    }

    fn request(&self, url: &str) {
        let scope = self.zone_id.as_scope();
        let Ok(parsed) = Url::parse(url) else {
            gosub_shared::subresource::abandon(scope, url);
            return;
        };

        let document = self.document.read().clone();
        // The same refusal the document scan applies, and the reason this belongs on this
        // side of the boundary: a page from the network does not get to read the disk because
        // it named the file somewhere the scan could not see.
        if parsed.scheme() == "file" && document.as_ref().is_none_or(|(url, _)| url.scheme() != "file") {
            log::warn!("refusing file:// media {parsed} for a document that was not loaded from disk");
            gosub_shared::subresource::abandon(scope, url);
            return;
        }

        let mut headers = http::HeaderMap::new();
        if let Some(langs) = &self.accept_language {
            if let Ok(value) = langs.parse() {
                headers.insert(http::header::ACCEPT_LANGUAGE, value);
            }
        }
        if let Ok(value) = ResourceKind::Image.accept_header().parse() {
            headers.insert(http::header::ACCEPT, value);
        }

        let req_id = RequestId::new();
        REF_REGISTRY.register_request(req_id, ResourceKind::Image, Initiator::CSS);
        let mut builder = FetchRequest::builder(Method::GET, parsed)
            .with_req_id(req_id)
            .with_priority(Priority::Low)
            .with_initiator(Initiator::CSS.to_net())
            .with_kind(ResourceKind::Image.to_net())
            .with_headers(headers)
            .with_streaming(false)
            .with_auto_decode(true);
        if let Some((doc_url, reference)) = document {
            builder = builder
                .with_referrer(doc_url)
                .with_reference(REF_REGISTRY.to_net(reference));
        }
        let req = builder.build();

        let zone_id = self.zone_id;
        let io_tx = self.io_tx.clone();
        let url = url.to_string();
        // Spawned onto the runtime by handle, not with `spawn_named`: the caller is a plain
        // thread waiting on the handoff, with no runtime of its own, and `tokio::spawn`
        // panics when there is no runtime in context.
        self.runtime.spawn(async move {
            let delivered = match submit_to_io(zone_id, req, io_tx, None).await {
                Ok((_handle, rx)) => match rx.await {
                    Ok(FetchResult::Buffered { meta, body }) if meta.status == 200 && !body.is_empty() => {
                        Some((meta.content_type.clone(), body.to_vec()))
                    }
                    _ => None,
                },
                Err(e) => {
                    log::warn!("Failed to submit media request: {e:?}");
                    None
                }
            };
            match delivered {
                Some((content_type, bytes)) => gosub_shared::subresource::complete(scope, &url, content_type, bytes),
                // Always answered, one way or the other: a consumer waiting on this URL is
                // otherwise left waiting for bytes that are never coming.
                None => gosub_shared::subresource::abandon(scope, &url),
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gosub_shared::subresource::Scope;
    use std::time::{Duration, Instant};

    fn source(document: Option<&str>) -> EngineMediaSource {
        let (io_tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let source = EngineMediaSource::new(ZoneId::new(), io_tx, Handle::current(), None);
        source.set_document(
            document.and_then(|d| Url::parse(d).ok()),
            RequestReference::Navigation(crate::engine::types::NavigationId::new()),
        );
        source
    }

    /// The hand-off is process-wide and `clear()` empties all of it, so these take turns:
    /// run in parallel they wipe entries another test is waiting on, which shows up as a
    /// flake rather than as a failure anyone can read.
    fn exclusively<T>(body: impl FnOnce() -> T) -> T {
        static LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());
        let _guard = LOCK.lock();
        gosub_shared::subresource::clear();
        body()
    }

    /// Refused *and answered*: `take` returning `None` proves nothing on its own, because a
    /// request that was silently dropped reads exactly the same way five seconds later. The
    /// clock is the assertion.
    fn refusal_is_immediate(source: &EngineMediaSource, scope: Scope, url: &str) {
        gosub_shared::subresource::begin(scope, url);
        source.request(url);

        let started = Instant::now();
        assert!(gosub_shared::subresource::take(scope, url).is_none(), "must not load");
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "the refusal has to answer the hand-off, not leave the consumer to time out"
        );
    }

    /// The rule the document scan applies, applied here too, because this is the other way a
    /// request reaches the network: an image named somewhere the scan could not read it. A
    /// page served over http rendering a local file was reachable through exactly this gap.
    #[tokio::test(flavor = "current_thread")]
    async fn a_document_from_the_network_may_not_ask_for_a_local_file() {
        exclusively(|| {
            let source = source(Some("http://example.com/page.html"));
            refusal_is_immediate(&source, source.scope(), "file:///etc/hostname");
        });
    }

    /// A store with no document yet is in no position to allow it either.
    #[tokio::test(flavor = "current_thread")]
    async fn a_source_with_no_document_may_not_ask_for_a_local_file() {
        exclusively(|| {
            let source = source(None);
            refusal_is_immediate(&source, source.scope(), "file:///etc/hostname");
        });
    }
}
