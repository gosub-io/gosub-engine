//! The engine's [`ResourceLoader`]: a blocking fetch that goes through the I/O
//! runtime instead of opening a socket where it is called.
//!
//! ## The blocking wait, and what it costs

use crate::engine::types::IoChannel;
use crate::engine::types::RequestId;
use crate::events::IoCommand;
use crate::net::types::{FetchHandle, FetchRequest, FetchResult};
use crate::tab::TabId;
use crate::zone::ZoneId;
use gosub_interface::resource_loader::{LoadError, LoadedResource, ResourceLoader};
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
    /// Cancelled when the work that needed these resources is abandoned — a
    /// navigation superseded, a tab closed. Without it a cancelled page's
    /// stylesheets and fonts would keep downloading.
    cancel: CancellationToken,
}

impl BrokeredLoader {
    pub fn new(zone_id: ZoneId, tab_id: Option<TabId>, io_tx: IoChannel) -> Self {
        Self {
            zone_id,
            tab_id,
            io_tx,
            cancel: CancellationToken::new(),
        }
    }

    /// Tie these loads to `parent`, so cancelling the work that wanted them
    /// cancels the fetches too.
    pub fn with_cancel(mut self, parent: &CancellationToken) -> Self {
        self.cancel = parent.child_token();
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
        if crate::telemetry::enabled() {
            let (outcome, status, bytes) = match &result {
                Ok(resource) => ("ok", Some(resource.status), resource.body.len()),
                Err(LoadError::UnsupportedUrl(_)) => ("refused-scheme", None, 0),
                Err(LoadError::TimedOut) => ("timeout", None, 0),
                Err(LoadError::Failed(_)) => ("failed", None, 0),
            };
            crate::telemetry::emit(
                "net.load",
                serde_json::json!({
                    "url": url.as_str(),
                    "tab": self.tab_id.map(|t| t.to_string()),
                    "outcome": outcome,
                    "status": status,
                    "bytes": bytes,
                    "duration_us": started.elapsed().as_micros() as u64,
                    "error": result.as_ref().err().map(|e| e.to_string()),
                }),
            );
        }
        result
    }
}

impl BrokeredLoader {
    fn load_inner(&self, url: &Url) -> Result<LoadedResource, LoadError> {
        if !matches!(url.scheme(), "http" | "https") {
            return Err(LoadError::UnsupportedUrl(url.to_string()));
        }
        warn_if_current_thread_runtime();

        let req = FetchRequest::builder(Method::GET, url.clone())
            .with_req_id(RequestId::new())
            .with_streaming(false)
            .with_auto_decode(true)
            .build();

        let handle = FetchHandle {
            req_id: req.req_id,
            cancel: self.cancel.child_token(),
        };

        // A std channel, not a tokio one: the receiver blocks a plain thread and
        // must not need a runtime of its own to be woken.
        let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel::<FetchResult>(1);
        let (io_tx, io_rx) = tokio::sync::oneshot::channel::<FetchResult>();
        crate::util::spawn_named("brokered-load", async move {
            if let Ok(result) = io_rx.await {
                let _ = reply_tx.send(result);
            }
        });

        self.io_tx
            .send(IoCommand::Fetch {
                zone_id: self.zone_id,
                tab_id: self.tab_id,
                req,
                handle,
                reply_tx: io_tx,
            })
            .map_err(|_| LoadError::Failed("the I/O runtime has shut down".into()))?;

        let result = reply_rx.recv_timeout(BROKER_REPLY_TIMEOUT).map_err(|_| {
            log::warn!("brokered load of {url} produced no reply within {BROKER_REPLY_TIMEOUT:?}");
            LoadError::TimedOut
        })?;

        into_loaded(result)
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
