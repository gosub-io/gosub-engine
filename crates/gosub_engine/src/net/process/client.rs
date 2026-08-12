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

/// The argv role name the broker re-execs itself with.
pub const NET_ROLE: &str = "net";

/// How long a caller waits for a reply before giving up on the network process.
const REPLY_TIMEOUT: Duration = Duration::from_secs(120);

/// How long to wait for the child to identify itself. Short: it answers before
/// doing any work, so anything slower means it is not a network process at all.
const READY_TIMEOUT: Duration = Duration::from_secs(10);

/// A running network process and the link to it.
pub struct NetProcess {
    tx: Mutex<EndpointTx>,
    pending: Arc<Mutex<HashMap<RequestTag, std::sync::mpsc::SyncSender<FetchOutcome>>>>,
    next_tag: AtomicU64,
    child: Mutex<Option<gosub_sandbox::spawn::Child>>,
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

        let child = gosub_sandbox::spawn::spawn(
            &exe,
            &[crate::child_process::ROLE_FLAG, NET_ROLE],
            theirs,
            // The one component that keeps its network namespace: isolating it
            // would leave the engine unable to reach anything at all.
            gosub_sandbox::NamespaceIsolation::None,
            gosub_sandbox::spawn::ContainerProfile {
                name: "gosub-net",
                internet: true,
                fs_grant: None,
            },
        )?;

        if let Err(e) = gosub_sandbox::confine_spawned_child(&child) {
            log::warn!("could not apply parent-side confinement to the network process: {e}");
        }

        let endpoint = Endpoint::from_channel(ours)?;
        let (tx, mut rx) = endpoint.split();

        let pending: Arc<Mutex<HashMap<RequestTag, std::sync::mpsc::SyncSender<FetchOutcome>>>> =
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
                                let _ = waiter.send(outcome);
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
            tx: Mutex::new(tx),
            pending,
            next_tag: AtomicU64::new(1),
            child: Mutex::new(Some(child)),
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

    /// Send a request and block until the network process answers.
    pub fn fetch(
        &self,
        url: String,
        method: String,
        headers: Vec<(String, String)>,
        body: Option<Vec<u8>>,
    ) -> FetchOutcome {
        let tag = self.next_tag.fetch_add(1, Ordering::Relaxed);
        let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel::<FetchOutcome>(1);
        self.pending.lock().insert(tag, reply_tx);

        let msg = ToNet::Fetch(NetFetch {
            tag,
            url,
            method,
            headers,
            body,
        });
        if let Err(e) = self.tx.lock().send(&msg) {
            self.pending.lock().remove(&tag);
            return FetchOutcome::Error(format!("could not reach the network process: {e}"));
        }

        match reply_rx.recv_timeout(REPLY_TIMEOUT) {
            Ok(outcome) => outcome,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                FetchOutcome::Error("the network process exited".into())
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                self.pending.lock().remove(&tag);
                FetchOutcome::Error("the network process did not answer".into())
            }
        }
    }

    /// Ask the process to stop, then make sure it has.
    pub fn shutdown(&self) {
        let _ = self.tx.lock().send(&ToNet::Shutdown);

        let Some(mut child) = self.child.lock().take() else {
            return;
        };
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
pub fn outcome_to_result(outcome: FetchOutcome) -> Result<crate::net::types::FetchResult, NetError> {
    let FetchOutcome::Ok {
        status,
        status_text,
        final_url,
        headers,
        body,
    } = outcome
    else {
        let FetchOutcome::Error(e) = outcome else {
            // The `else` above already excluded `Ok`.
            return Err(net_error("unexpected outcome from the network process"));
        };
        return Err(net_error(e));
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

    Ok(crate::net::types::FetchResult::Buffered {
        meta: crate::net::types::FetchResultMeta {
            final_url,
            status,
            status_text,
            headers: header_map,
            content_length,
            content_type,
            has_body: !body.is_empty(),
        },
        body: bytes::Bytes::from(body),
    })
}

/// A failure that came from (or about) the network process, as the engine's own
/// error type. `Other` because the cause is a broker↔child protocol problem
/// rather than any of the transport-specific variants.
pub fn net_error(msg: impl Into<String>) -> NetError {
    NetError::Other(std::sync::Arc::new(anyhow::anyhow!(msg.into())))
}
