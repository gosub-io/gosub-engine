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
    /// The navigation these requests belong to, which is how the hand-off tells one page's
    /// preloads from another's. `None` until a navigation commits, and nothing asks for an
    /// image before then.
    fn scope(&self) -> Option<gosub_shared::subresource::Scope> {
        match self.document.read().as_ref() {
            Some((_, RequestReference::Navigation(nav_id))) => Some(nav_id.as_scope()),
            _ => None,
        }
    }

    fn request(&self, url: &str) {
        // Nothing to deposit into and nobody waiting: a consumer keyed by a navigation could
        // not have found these bytes anyway, so this is a request nobody asked for.
        let Some(scope) = self.scope() else {
            log::warn!("media request for {url} with no navigation to attribute it to");
            return;
        };
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
            let scope = source.scope().expect("a committed navigation has a scope");
            refusal_is_immediate(&source, scope, "file:///etc/hostname");
        });
    }

    /// What keys the hand-off is the navigation, not the zone: one page's preloads are not
    /// an answer to another page's request for the same URL, even in the same zone, because
    /// the two do not share a request context. A second `set_document` is a second page.
    #[tokio::test(flavor = "current_thread")]
    async fn a_second_navigation_does_not_inherit_the_first_ones_preloads() {
        exclusively(|| {
            let source = source(Some("http://example.com/page.html"));
            let first = source.scope().expect("a committed navigation has a scope");
            let url = "http://example.com/hero.png";

            gosub_shared::subresource::begin(first, url);
            gosub_shared::subresource::complete(first, url, Some("image/png".into()), b"PNG".to_vec());

            // The same tab, the same zone, the next page.
            source.set_document(
                Url::parse("http://example.com/next.html").ok(),
                RequestReference::Navigation(crate::engine::types::NavigationId::new()),
            );
            let second = source.scope().expect("and so does the one after it");
            assert_ne!(first, second, "a new navigation is a new scope");
            assert!(
                gosub_shared::subresource::take(second, url).is_none(),
                "the new page must fetch for itself rather than inherit the old page's bytes"
            );

            // Still there for the page they were fetched for.
            assert!(gosub_shared::subresource::take(first, url).is_some());
        });
    }

    /// A source with no document is in no position to allow anything: it cannot say which
    /// page a request belongs to, so it makes none at all -- stricter than the `file://`
    /// refusal it used to answer this case with.
    #[tokio::test(flavor = "current_thread")]
    async fn a_source_with_no_document_asks_for_nothing() {
        exclusively(|| {
            let source = source(None);
            assert!(source.scope().is_none());

            // Nothing announced and nothing deposited: asking under any scope finds an empty
            // store and returns at once, rather than an entry left in flight for a fetch that
            // is never going to happen.
            let scope: gosub_shared::subresource::Scope = 99;
            let url = "file:///etc/hostname";
            source.request(url);

            let started = Instant::now();
            assert!(gosub_shared::subresource::take(scope, url).is_none());
            assert!(started.elapsed() < Duration::from_secs(1));
        });
    }
}
