//! The broker's side of the network process: spawn it, talk to it, notice when
//! it dies.

use crate::net::process::protocol::{CookieScope, FetchOutcome, FromNet, NetFetch, RequestTag, ToNet};
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
    /// Serve through the strict fetcher (see `net::ssrf`).
    pub refuse_private: bool,
    /// Deliver the body through a ring as it arrives, where the link can carry one.
    pub streaming: bool,
    /// Whose cookies to attach, resolved by the network process against the
    /// cookie vault. `None` when the broker attached the header itself.
    pub cookies: Option<CookieScope>,
}

impl Outbound {
    /// A plain GET with no body and no special handling.
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            method: "GET".into(),
            headers: Vec::new(),
            body: None,
            refuse_private: false,
            streaming: false,
            cookies: None,
        }
    }
}

/// The ring's descriptor, where one can exist; nothing where it cannot.
#[cfg(target_os = "linux")]
type RingFd = std::os::fd::OwnedFd;
#[cfg(not(target_os = "linux"))]
type RingFd = std::convert::Infallible;

/// A reply as the broker sees it: the wire outcome plus, for a streamed body,
/// the ring fd that followed it on the link.
#[derive(Debug)]
pub struct NetReply {
    pub outcome: FetchOutcome,
    pub ring: Option<RingFd>,
}

impl NetReply {
    fn error(msg: impl Into<String>) -> Self {
        Self {
            outcome: FetchOutcome::Error(msg.into()),
            ring: None,
        }
    }
}

#[cfg(target_os = "linux")]
fn recv_ring(rx: &mut gosub_ipc::EndpointRx) -> std::io::Result<RingFd> {
    rx.recv_fd()
}

#[cfg(not(target_os = "linux"))]
fn recv_ring(_rx: &mut gosub_ipc::EndpointRx) -> std::io::Result<RingFd> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "no fd passing on this platform",
    ))
}

/// The argv role name the broker re-execs itself with.
pub const NET_ROLE: &str = "net";

/// The network process's end of a line to the cookie vault (Linux); an opaque
/// channel elsewhere, never created.
pub struct VaultLine(pub gosub_ipc::channel::Channel);

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
    /// The child holds a direct line to the cookie vault: requests may carry a
    /// cookie scope instead of a header.
    vault_linked: bool,
    /// Who is waiting for an audit report, if anyone.
    audit_waiter: AuditWaiter,
}

type AuditWaiter = Arc<Mutex<Option<std::sync::mpsc::SyncSender<Option<gosub_sandbox::audit::AuditReport>>>>>;

impl std::fmt::Debug for NetProcess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NetProcess").finish_non_exhaustive()
    }
}

impl NetProcess {
    /// Re-exec this binary as the network process and connect to it. With
    /// `vault`, the child also inherits its line to the cookie vault.
    pub fn spawn(vault: Option<VaultLine>) -> anyhow::Result<Self> {
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

        // The vault line rides along as an extra inherited fd, named in argv
        // before the primary link (which `spawn` appends).
        let vault_spec = vault.as_ref().map(|line| line.0.to_argv());
        let mut args: Vec<&str> = vec![crate::child_process::ROLE_FLAG, NET_ROLE];
        if let Some(spec) = vault_spec.as_deref() {
            args.push(spec);
        }
        #[cfg(target_os = "linux")]
        let extra_fds: Vec<i32> = vault.iter().map(|line| line.0.raw()).collect();
        #[cfg(not(target_os = "linux"))]
        let extra_fds: Vec<i32> = Vec::new();

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
                extra_fds: &extra_fds,
                // A multi-thread runtime plus its blocking pool.
                max_tasks: 1024,
                file_size_limit: None,
            },
        )?;
        drop(vault); // the child holds its copy of the vault line

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
        let audit_waiter: AuditWaiter = Arc::new(Mutex::new(None));
        let audit_reply = Arc::clone(&audit_waiter);
        std::thread::Builder::new()
            .name("net-process-reader".into())
            .spawn(move || {
                while let Ok(msg) = rx.recv::<FromNet>() {
                    match msg {
                        FromNet::Pong => {
                            let _ = ready_tx.send(());
                        }
                        FromNet::Audit(report) => {
                            if let Some(waiter) = audit_reply.lock().take() {
                                let _ = waiter.send(report);
                            }
                        }
                        FromNet::Reply { tag, outcome } => {
                            // A streamed head is followed by its ring fd; take it
                            // now, before the next message, whoever is waiting.
                            let reply = match outcome {
                                FetchOutcome::Streaming { .. } => match recv_ring(&mut rx) {
                                    Ok(ring) => NetReply {
                                        outcome,
                                        ring: Some(ring),
                                    },
                                    Err(e) => NetReply::error(format!("body stream fd did not arrive: {e}")),
                                },
                                outcome => NetReply { outcome, ring: None },
                            };
                            if let Some(waiter) = waiters.lock().remove(&tag) {
                                let _ = waiter.send(reply);
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
            vault_linked: !extra_fds.is_empty(),
            audit_waiter,
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

    /// Whether the child resolves cookies against the vault itself.
    pub fn vault_linked(&self) -> bool {
        self.vault_linked
    }

    /// The escape audit, run inside the network process. Blocking.
    pub fn audit(&self) -> anyhow::Result<Option<gosub_sandbox::audit::AuditReport>> {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        *self.audit_waiter.lock() = Some(tx);
        self.tx.lock().send(&ToNet::Audit)?;
        rx.recv_timeout(Duration::from_secs(30))
            .map_err(|_| anyhow::anyhow!("the network process did not answer the audit"))
    }

    /// Hand the child a new line to a respawned vault, over its own link. The
    /// fd goes twice: the child may not `dup`, and an endpoint is two halves.
    #[cfg(target_os = "linux")]
    pub fn relink_vault(&self, line: VaultLine) {
        let mut tx = self.tx.lock();
        if tx.send(&ToNet::VaultLine).is_err() || tx.send_fd(line.0.raw()).is_err() || tx.send_fd(line.0.raw()).is_err()
        {
            log::warn!("could not hand the network process its new vault line");
        }
        // `line` drops here: the child holds its duplicates.
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
            refuse_private,
            streaming,
            cookies,
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
            refuse_private,
            streaming,
            cookies,
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
            FetchOutcome::Ok { final_url, .. } | FetchOutcome::Streaming { final_url, .. } => final_url,
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
        } => (status, status_text, final_url, headers, Body::Whole(body)),
        FetchOutcome::Streaming {
            status,
            status_text,
            final_url,
            headers,
            peek,
        } => {
            let Some(ring) = reply.ring else {
                return Err(net_error("streamed reply without its ring"));
            };
            (status, status_text, final_url, headers, Body::Ring { peek, ring })
        }
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
    Ok(match body {
        Body::Whole(body) => crate::net::types::FetchResult::Buffered {
            meta: meta(!body.is_empty()),
            body: bytes::Bytes::from(body),
        },
        Body::Ring { peek, ring } => crate::net::types::FetchResult::Stream {
            meta: meta(true),
            peek_buf: gosub_sonar::types::PeekBuf::from_vec(peek),
            shared: drain_ring(ring),
        },
    })
}

/// A reply's body as it came off the wire.
enum Body {
    Whole(Vec<u8>),
    /// The head arrived; the body streams through this ring.
    Ring {
        peek: Vec<u8>,
        ring: RingFd,
    },
}

/// Per-subscriber queue for a ring-fed body, in chunks: the same order of
/// magnitude gosub-sonar uses for its own streamed responses.
const RING_BODY_QUEUE: usize = 64;

/// Feed a [`SharedBody`] from a ring on a thread of its own: the ring's reads
/// block (bounded by its stall timeout), so this is not runtime work. The
/// body ends when the producer finishes; a producer that aborts or stalls
/// ends it with an error.
///
/// [`SharedBody`]: gosub_sonar::net::shared_body::SharedBody
fn drain_ring(ring: RingFd) -> Arc<gosub_sonar::net::shared_body::SharedBody> {
    use gosub_sonar::net::shared_body::SharedBody;
    let shared = Arc::new(SharedBody::new(RING_BODY_QUEUE));
    let sink = Arc::clone(&shared);
    let spawned = std::thread::Builder::new()
        .name("net-ring-consumer".into())
        .spawn(move || {
            #[cfg(target_os = "linux")]
            {
                let mut consumer = match gosub_ipc::ring::RingConsumer::open(ring) {
                    Ok(c) => c,
                    Err(e) => {
                        sink.error(net_error(format!("body stream could not be opened: {e}")));
                        return;
                    }
                };
                let mut buf = vec![0u8; 64 * 1024];
                let mut total: u64 = 0;
                loop {
                    match consumer.read(&mut buf) {
                        Ok(0) => {
                            sink.finish();
                            return;
                        }
                        Ok(n) => {
                            total += n as u64;
                            if total > gosub_ipc::ring::MAX_BODY_LEN {
                                sink.error(net_error("body stream exceeded the size cap"));
                                return;
                            }
                            sink.push(bytes::Bytes::copy_from_slice(&buf[..n]));
                        }
                        Err(e) => {
                            sink.error(net_error(format!("body stream failed: {e}")));
                            return;
                        }
                    }
                }
            }
            #[cfg(not(target_os = "linux"))]
            {
                let _ = ring;
                sink.error(net_error("body streams are not carried on this platform"));
            }
        });
    if spawned.is_err() {
        shared.error(net_error("could not start the body stream consumer"));
    }
    shared
}

/// A failure that came from (or about) the network process, as the engine's own
/// error type. `Other` because the cause is a broker↔child protocol problem
/// rather than any of the transport-specific variants.
pub fn net_error(msg: impl Into<String>) -> NetError {
    NetError::Other(std::sync::Arc::new(anyhow::anyhow!(msg.into())))
}
