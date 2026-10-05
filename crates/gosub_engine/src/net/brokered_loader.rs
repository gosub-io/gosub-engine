//! The broker's [`ResourceLoader`]: a blocking fetch that goes through the I/O runtime,
//! used to answer a renderer process's resource requests.
//!
//! ## The blocking wait, and what it costs
//!
//! [`load`](ResourceLoader::load) parks the calling thread until the I/O runtime replies. The
//! reply is produced by a task on the engine's runtime, so the caller must not be that
//! runtime's only worker: on a current-thread runtime the task can never run and every load
//! times out after [`BROKER_REPLY_TIMEOUT`]. Call it from a blocking-pool thread or a plain
//! thread.

use crate::engine::events::IoCommand;
use crate::engine::types::IoChannel;
use crate::engine::types::RequestId;
use crate::net::resource_loader::{LoadError, LoadedResource, ResourceLoader};
use crate::net::types::{FetchHandle, FetchRequest, FetchResult};
use crate::tab::TabId;
use crate::zone::ZoneId;
use http::Method;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use url::Url;

/// How long a brokered load waits before giving up.
const BROKER_REPLY_TIMEOUT: Duration = Duration::from_secs(60);

/// Fetches resources for one tab through the engine's I/O runtime.
#[derive(Debug, Clone)]
pub struct BrokeredLoader {
    zone_id: ZoneId,
    /// The tab these loads are attributed to, so the I/O side applies its
    /// cookies. `None` for engine-internal loads that belong to no tab.
    tab_id: Option<TabId>,
    io_tx: IoChannel,
    /// Cancelled when the work that needed these resources is abandoned - a
    /// navigation superseded, a tab closed. Without it a cancelled page's
    /// stylesheets and fonts would keep downloading.
    cancel: CancellationToken,
    /// The runtime this loader relays replies on, captured at construction.
    /// Loads are issued from plain threads too (the remote media cache's fetch
    /// threads), where `Handle::try_current` finds nothing to spawn on.
    runtime: Option<tokio::runtime::Handle>,
    /// The document the loads are made for ([`ResourceLoader::for_document`]):
    /// its `Referer`, which is also what the I/O side judges the request's
    /// policies by, and whether `file:` neighbours may be loaded.
    document: Option<Url>,
    /// The tab's `Accept-Language`, as the in-process fetches send it.
    accept_language: Option<String>,
}

impl BrokeredLoader {
    pub(crate) fn new(zone_id: ZoneId, tab_id: Option<TabId>, io_tx: IoChannel) -> Self {
        Self {
            zone_id,
            tab_id,
            io_tx,
            cancel: CancellationToken::new(),
            runtime: tokio::runtime::Handle::try_current().ok(),
            document: None,
            accept_language: None,
        }
    }

    /// Tie these loads to `parent`, so cancelling the work that wanted them
    /// cancels the fetches too.
    pub fn with_cancel(mut self, parent: &CancellationToken) -> Self {
        self.cancel = parent.child_token();
        self
    }

    /// Send `Accept-Language` on every load, like the tab's own fetches.
    pub fn with_accept_language(mut self, langs: Option<String>) -> Self {
        self.accept_language = langs;
        self
    }

    /// Share this loader with a subsystem that stores it type-erased.
    pub fn shared(self) -> Arc<dyn ResourceLoader> {
        Arc::new(self)
    }
}

impl ResourceLoader for BrokeredLoader {
    fn load(&self, url: &Url) -> Result<LoadedResource, LoadError> {
        let started = std::time::Instant::now();
        let result = self.load_inner(url);
        crate::telemetry::net_load(url.as_str(), self.tab_id, started, result.as_ref().ok());
        result.and_then(into_loaded)
    }

    fn for_document(&self, url: Option<&Url>) -> Option<Arc<dyn ResourceLoader>> {
        Some(Arc::new(Self {
            document: url.cloned(),
            ..self.clone()
        }))
    }
}

impl BrokeredLoader {
    fn load_inner(&self, url: &Url) -> Result<FetchResult, LoadError> {
        let document = self.document.clone();
        // `data:` carries its own bytes, and the I/O runtime answers it without
        // the network. `file:` only for a document that itself came from
        // disk - the same rule the in-process media source applies - and
        // then under the I/O side's own file policy, which reads the
        // `Referer` set below: a renderer asks through here, and local files
        // are the broker's to open.
        let from_disk = document.as_ref().is_some_and(|doc| doc.scheme() == "file");
        let served = match url.scheme() {
            "http" | "https" | "data" => true,
            "file" => from_disk,
            _ => false,
        };
        if !served {
            return Err(LoadError::UnsupportedUrl(url.to_string()));
        }
        warn_if_current_thread_runtime();

        // The request a page's own fetch would have made: `Referer` (what a
        // hotlink-protected image or a `file:` load is judged by) and the
        // tab's language preference. A subresource on a page's behalf: the
        // I/O side applies the private-network and opaque-response policies.
        let mut headers = http::HeaderMap::new();
        if let Some(langs) = &self.accept_language {
            if let Ok(value) = langs.parse() {
                headers.insert(http::header::ACCEPT_LANGUAGE, value);
            }
        }
        let mut builder = FetchRequest::builder(Method::GET, url.clone())
            .with_req_id(RequestId::new())
            .with_headers(headers)
            .with_kind(gosub_sonar::net::types::ResourceKind::Asset)
            .with_initiator(gosub_sonar::net::types::Initiator::Application)
            .with_streaming(false)
            .with_auto_decode(true);
        if let Some(doc) = document {
            builder = builder.with_referrer(doc);
        }
        let req = builder.build();

        let cancel = self.cancel.child_token();
        let handle = FetchHandle {
            req_id: req.req_id,
            cancel: cancel.clone(),
        };

        // A std channel, not a tokio one: the receiver blocks a plain thread and
        // must not need a runtime of its own to be woken.
        let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel::<FetchResult>(1);
        let (io_tx, io_rx) = tokio::sync::oneshot::channel::<FetchResult>();
        // The relay runs on whichever runtime is at hand: the current one, or the
        // one captured at construction when this load comes from a plain thread.
        // With neither there is nothing to run it on, which is an error, not a
        // panic in `tokio::spawn`.
        let relay = async move {
            if let Ok(result) = io_rx.await {
                let _ = reply_tx.send(result);
            }
        };
        match tokio::runtime::Handle::try_current()
            .ok()
            .or_else(|| self.runtime.clone())
        {
            Some(handle) => {
                handle.spawn(relay);
            }
            None => return Err(LoadError::Failed("no runtime to relay the reply on".into())),
        }

        self.io_tx
            .send(IoCommand::Fetch {
                zone_id: self.zone_id,
                tab_id: self.tab_id,
                req,
                handle,
                reply_tx: io_tx,
            })
            .map_err(|_| LoadError::Failed("the I/O runtime has shut down".into()))?;

        // The blocking wait must not hold this worker's scheduler core: the
        // send above has just woken the I/O task into this worker's LIFO slot,
        // which no other worker can steal - blocking here directly would trap
        // the very task that produces the reply until the timeout expires.
        // `block_in_place` hands the core (and that slot) to another thread
        // for the duration. Outside a multi-thread runtime worker there is no
        // core to hand over, and a plain blocking wait is correct.
        let wait = || reply_rx.recv_timeout(BROKER_REPLY_TIMEOUT);
        let result = match tokio::runtime::Handle::try_current() {
            Ok(h) if h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
                tokio::task::block_in_place(wait)
            }
            _ => wait(),
        };
        result.map_err(|_| {
            log::warn!("brokered load of {url} produced no reply within {BROKER_REPLY_TIMEOUT:?}");
            // Nobody is waiting anymore: free the in-flight slot and the child's work.
            cancel.cancel();
            LoadError::TimedOut
        })
    }
}

fn into_loaded(result: FetchResult) -> Result<LoadedResource, LoadError> {
    match result {
        FetchResult::Buffered { meta, body } => Ok(LoadedResource {
            status: meta.status,
            content_type: meta.content_type,
            body,
        }),
        // Brokered loads are submitted buffered, so a stream here means the
        // request was rewritten somewhere in between.
        FetchResult::Stream { meta, .. } => Err(LoadError::Failed(format!(
            "expected a buffered body for {}, got a stream",
            meta.final_url
        ))),
        FetchResult::Error(e) => Err(LoadError::Failed(e.to_string())),
    }
}

/// Warn once if this thread cannot be blocked safely.
fn warn_if_current_thread_runtime() {
    static WARNED: AtomicBool = AtomicBool::new(false);

    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        // Not on a runtime thread at all: blocking here starves nothing.
        return;
    };
    if handle.runtime_flavor() != tokio::runtime::RuntimeFlavor::CurrentThread {
        return;
    }
    if WARNED.swap(true, Ordering::Relaxed) {
        return;
    }
    log::warn!(
        "a resource load is blocking a current-thread tokio runtime; the engine's I/O task \
         cannot run while it waits, so this load will time out. Drive the engine on a \
         multi-threaded runtime for external stylesheets, web fonts and images to load."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A renderer's stylesheet or web font can be a `data:` URL; the loader
    /// hands it to the I/O runtime like any fetch. `file:` is still refused.
    #[test]
    fn a_data_url_loads_and_a_file_url_does_not() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let _in_rt = rt.enter();
        let (event_tx, _events) = tokio::sync::broadcast::channel(16);
        let ctx = Arc::new(crate::engine::EngineContext {
            event_tx,
            ..Default::default()
        });
        let io = crate::net::io_runtime::spawn_io_thread(ctx);
        let loader = BrokeredLoader::new(ZoneId::new(), None, io.subscribe());

        let loaded = loader
            .load(&Url::parse("data:text/plain,hello").unwrap())
            .expect("a data: URL loads");
        assert_eq!(&loaded.body[..], b"hello");

        let refused = loader.load(&Url::parse("file:///etc/hostname").unwrap());
        assert!(matches!(refused, Err(LoadError::UnsupportedUrl(_))), "{refused:?}");

        // A document loaded from disk may load its neighbours, like in-process.
        let dir = std::env::temp_dir().join(format!("gosub-brokered-file-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("page.css"), b"body{}").unwrap();
        let page = Url::from_file_path(dir.join("index.html")).unwrap();
        let for_page = loader
            .for_document(Some(&page))
            .expect("a brokered loader binds to a document");
        let neighbour = Url::from_file_path(dir.join("page.css")).unwrap();
        let loaded = for_page
            .load(&neighbour)
            .expect("a file: neighbour of a file: document loads");
        assert_eq!(&loaded.body[..], b"body{}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
