//! The broker's side of the network process: spawn it, talk to it, notice when
//! it dies.

use crate::net::process::protocol::{FetchOutcome, FromNet, NetFetch, RequestTag, ToNet};
use crate::net::types::NetError;
use gosub_ipc::{Endpoint, EndpointTx};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// One request for the network process, as the broker hands it over.
#[derive(Debug)]
pub struct Outbound {
    pub url: String,
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
}

impl Outbound {
    /// A plain GET with no body and no special handling.
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            method: "GET".into(),
            headers: Vec::new(),
            body: None,
        }
    }
}

/// A reply as the broker sees it: the wire outcome.
#[derive(Debug)]
pub struct NetReply {
    pub outcome: FetchOutcome,
}

impl NetReply {
    fn error(msg: impl Into<String>) -> Self {
        Self {
            outcome: FetchOutcome::Error(msg.into()),
        }
    }
}

/// The argv role name the broker re-execs itself with.
pub const NET_ROLE: &str = "net";

/// How long a caller waits for a reply before giving up on the network process.
const REPLY_TIMEOUT: Duration = Duration::from_secs(120);

/// How many requests may be in flight in the network process at once. Callers
/// past this bound wait for a slot rather than being refused: subresource
/// bursts are the normal case, and backpressure degrades better than errors.
const MAX_INFLIGHT: usize = 16;

/// How long to wait for the child to identify itself. Short: it answers before
/// doing any work, so anything slower means it is not a network process at all.
const READY_TIMEOUT: Duration = Duration::from_secs(10);

/// How long shutdown waits for the child to finish in-flight work and exit on
/// its own before killing it. A little longer than the child's own drain grace,
/// so a well-behaved child is never killed mid-drain.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

/// A running network process and the link to it.
pub struct NetProcess {
    tx: Arc<Mutex<EndpointTx>>,
    pending: Arc<Mutex<HashMap<RequestTag, tokio::sync::oneshot::Sender<NetReply>>>>,
    next_tag: AtomicU64,
    child: Mutex<Option<gosub_sandbox::spawn::Child>>,
    /// Bounds concurrent requests (see [`MAX_INFLIGHT`]).
    inflight: Arc<tokio::sync::Semaphore>,
}

impl std::fmt::Debug for NetProcess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NetProcess").finish_non_exhaustive()
    }
}

impl NetProcess {
    /// Re-exec this binary as the network process and connect to it.
    pub fn spawn() -> anyhow::Result<Self> {
        // A process that carries a child role but is running broker code got here
        // because the embedder never dispatched, so re-exec put it into its own
        // `main`. Spawning from here would do the same thing again, and again:
        // an unbounded chain of processes, each opening whatever the embedder
        // opens. Refuse, and name the omission.
        if crate::child_process::is_child_process() {
            anyhow::bail!(
                "this process was started as an engine child role but is running embedder startup, \
                 which means gosub_engine::child_process::dispatch() was not called at the top of \
                 main(); refusing to spawn further processes"
            );
        }

        let exe = std::env::current_exe()?;
        let (ours, theirs) = gosub_ipc::channel::Channel::pair()?;
        let args: [&str; 2] = [crate::child_process::ROLE_FLAG, NET_ROLE];

        let child = gosub_sandbox::spawn::spawn(
            &exe,
            &args,
            theirs,
            // The one component that keeps its network namespace.
            gosub_sandbox::NamespaceIsolation::KeepNetwork,
            gosub_sandbox::spawn::ContainerProfile {
                name: "gosub-net",
                internet: true,
                fs_grant: None,
                data_limit: None,
                extra_fds: &[],
                // A multi-thread runtime plus its blocking pool.
                max_tasks: 1024,
                file_size_limit: None,
            },
        )?;
        if let Err(e) = gosub_sandbox::confine_spawned_child(&child) {
            log::warn!("could not apply parent-side confinement to the network process: {e}");
        }

        let mut endpoint = Endpoint::from_channel(ours)?;
        // A child that stops reading must not pin blocking-pool threads forever.
        let _ = endpoint.tx.set_write_timeout(Some(REPLY_TIMEOUT));
        let (tx, mut rx) = endpoint.split();

        let pending: Arc<Mutex<HashMap<RequestTag, tokio::sync::oneshot::Sender<NetReply>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel::<()>(1);

        // A plain thread, not a task: it blocks on the link, and must keep
        // draining even when every runtime worker is busy waiting on a reply.
        let waiters = pending.clone();
        std::thread::Builder::new()
            .name("net-process-reader".into())
            .spawn(move || {
                while let Ok(msg) = rx.recv::<FromNet>() {
                    match msg {
                        FromNet::Pong => {
                            let _ = ready_tx.send(());
                        }
                        FromNet::Reply { tag, outcome } => {
                            if let Some(waiter) = waiters.lock().remove(&tag) {
                                let _ = waiter.send(NetReply { outcome });
                            }
                        }
                    }
                }
                // The link is gone, so no reply will ever arrive. Dropping the
                // senders wakes every waiter with a disconnect instead of leaving
                // them to time out one by one.
                waiters.lock().clear();
            })?;

        let net = Self {
            tx: Arc::new(Mutex::new(tx)),
            pending,
            next_tag: AtomicU64::new(1),
            child: Mutex::new(Some(child)),
            inflight: Arc::new(tokio::sync::Semaphore::new(MAX_INFLIGHT)),
        };

        // Confirm the child really is a network process before returning it as
        // one. Without this, a child that answers nothing (see `ToNet::Ping`)
        // would only be noticed when the first request timed out.
        net.tx.lock().send(&ToNet::Ping)?;
        match ready_rx.recv_timeout(READY_TIMEOUT) {
            Ok(()) => {}
            // The reader thread ended, so the link is gone: the child died
            // rather than went quiet. Report how it died, not a bogus timeout.
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                let fate = net
                    .child
                    .lock()
                    .take()
                    .map_or_else(|| "already reaped".to_string(), |mut c| c.wait_describe());
                anyhow::bail!("the network process died before answering ({fate})");
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                net.shutdown();
                anyhow::bail!("the spawned process did not answer as a network process within {READY_TIMEOUT:?}");
            }
        }

        Ok(net)
    }

    /// Send a request and wait for the network process to answer. Bounded by
    /// [`MAX_INFLIGHT`]; a caller past the bound waits for a slot. Cancelling
    /// `cancel` abandons the wait and tells the child to drop the request.
    pub async fn fetch(&self, out: Outbound, cancel: &CancellationToken) -> NetReply {
        let Outbound {
            url,
            method,
            headers,
            body,
        } = out;
        let permit = tokio::select! {
            _ = cancel.cancelled() => return NetReply::error("cancelled"),
            p = self.inflight.clone().acquire_owned() => p,
        };
        let Ok(_permit) = permit else {
            return NetReply::error("the network process is shutting down");
        };

        let tag = self.next_tag.fetch_add(1, Ordering::Relaxed);
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel::<NetReply>();
        self.pending.lock().insert(tag, reply_tx);
        let requested = url.clone();

        let msg = ToNet::Fetch(NetFetch {
            tag,
            url,
            method,
            headers,
            body,
        });
        // The link write can block on a full pipe (bodies can be large), so it
        // runs on a blocking thread rather than a runtime worker.
        let tx = self.tx.clone();
        let sent = tokio::task::spawn_blocking(move || tx.lock().send(&msg)).await;
        match sent {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                self.pending.lock().remove(&tag);
                return NetReply::error(format!("could not reach the network process: {e}"));
            }
            Err(e) => {
                self.pending.lock().remove(&tag);
                return NetReply::error(format!("could not dispatch to the network process: {e}"));
            }
        }

        tokio::select! {
            _ = cancel.cancelled() => {
                self.pending.lock().remove(&tag);
                self.send_cancel(tag);
                NetReply::error("cancelled")
            }
            reply = tokio::time::timeout(REPLY_TIMEOUT, reply_rx) => match reply {
                Ok(Ok(reply)) => Self::plausible_reply(reply, &requested),
                Ok(Err(_)) => NetReply::error("the network process exited"),
                Err(_) => {
                    self.pending.lock().remove(&tag);
                    // Without this the child would keep working the request (and
                    // holding its resources) long after anyone cared.
                    self.send_cancel(tag);
                    NetReply::error("the network process did not answer")
                }
            },
        }
    }

    /// The child's word on where a request ended is checked against what was
    /// asked: a `final_url` must be a web URL, and stay on the requested URL's
    /// scheme family - a redirect chain cannot land on `file:` or an internal
    /// page, whatever the child reports. Where it landed within the web is still
    /// the child's word; only the broker following redirects itself could fix that.
    fn plausible_reply(reply: NetReply, requested: &str) -> NetReply {
        let final_url = match &reply.outcome {
            FetchOutcome::Ok { final_url, .. } => final_url,
            FetchOutcome::Error(_) => return reply,
        };
        let Ok(parsed) = url::Url::parse(final_url) else {
            return NetReply::error(format!(
                "the network process reported an unparsable final url for {requested}"
            ));
        };
        if !matches!(parsed.scheme(), "http" | "https") {
            return NetReply::error(format!(
                "the network process reported a non-web final url ({}) for {requested}",
                parsed.scheme()
            ));
        }
        reply
    }

    /// Tell the child to drop a request nobody is waiting for anymore.
    /// Best-effort, on a blocking thread: the pipe write may block, and there
    /// is no reply to wait for - a child that already answered simply finds no
    /// waiter.
    fn send_cancel(&self, tag: RequestTag) {
        let tx = self.tx.clone();
        tokio::task::spawn_blocking(move || {
            let _ = tx.lock().send(&ToNet::Cancel(tag));
        });
    }

    /// Ask the process to stop, then make sure it has. The child drains its
    /// in-flight requests before exiting (see [`ToNet::Shutdown`]), so give it
    /// [`SHUTDOWN_GRACE`] to do that; kill only one that fails to.
    pub fn shutdown(&self) {
        let _ = self.tx.lock().send(&ToNet::Shutdown);

        let Some(mut child) = self.child.lock().take() else {
            return;
        };
        let deadline = std::time::Instant::now() + SHUTDOWN_GRACE;
        while std::time::Instant::now() < deadline {
            match child.try_wait() {
                Ok(true) => return,
                Ok(false) => std::thread::sleep(Duration::from_millis(50)),
                // The child cannot be observed; fall through to the kill.
                Err(_) => break,
            }
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

impl Drop for NetProcess {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Rebuild the engine's own result type from what came back over the wire.
pub fn outcome_to_result(reply: NetReply) -> Result<crate::net::types::FetchResult, NetError> {
    let (status, status_text, final_url, headers, body) = match reply.outcome {
        FetchOutcome::Ok {
            status,
            status_text,
            final_url,
            headers,
            body,
        } => (status, status_text, final_url, headers, body),
        FetchOutcome::Error(e) => return Err(net_error(e)),
    };

    let final_url = url::Url::parse(&final_url).map_err(|e| net_error(format!("bad final url: {e}")))?;

    let mut header_map = http::HeaderMap::new();
    for (name, value) in &headers {
        let parsed = http::header::HeaderName::from_bytes(name.as_bytes())
            .ok()
            .zip(value.parse().ok());
        if let Some((name, value)) = parsed {
            header_map.append(name, value);
        }
    }

    let content_type = header_map
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let content_length = header_map
        .get(http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok());

    let meta = |has_body: bool| crate::net::types::FetchResultMeta {
        final_url,
        status,
        status_text,
        headers: header_map,
        content_length,
        content_type,
        has_body,
        tainting: gosub_sonar::ResponseTainting::Basic,
    };
    Ok(crate::net::types::FetchResult::Buffered {
        meta: meta(!body.is_empty()),
        body: bytes::Bytes::from(body),
    })
}

/// A failure that came from (or about) the network process, as the engine's own
/// error type. `Other` because the cause is a broker↔child protocol problem
/// rather than any of the transport-specific variants.
pub fn net_error(msg: impl Into<String>) -> NetError {
    NetError::Other(std::sync::Arc::new(anyhow::anyhow!(msg.into())))
}
