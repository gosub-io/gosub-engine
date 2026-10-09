//! The broker's side of the network process: spawn it, talk to it, notice when
//! it dies.

use crate::net::emitter::NetObserver;
use crate::net::process::protocol::{
    cut_string, rebuild_headers, CookieScope, FetchOutcome, FromNet, HeaderList, NetEventWire, NetFetch,
    RequestContext, RequestTag, ToNet, MAX_EVENT_STRING, MAX_REPLY_HEADERS, MAX_REPLY_HEADER_BYTES, MAX_REPLY_URL,
};
use crate::net::ssrf::AddressSpace;
use crate::net::types::NetError;
use gosub_ipc::{Endpoint, EndpointTx};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// One request for the network process, as the broker hands it over.
#[derive(Debug)]
pub struct Outbound {
    pub url: String,
    pub method: String,
    pub headers: HeaderList,
    pub body: Option<Vec<u8>>,
    /// The most private address space the request may reach: its document's
    /// (see `net::ssrf`); `Loopback` refuses nothing.
    pub reach: AddressSpace,
    /// Deliver the body through a ring as it arrives, where the link can carry one.
    pub streaming: bool,
    /// Whose cookies to attach, resolved by the network process against the
    /// cookie vault. `None` when the broker attached the header itself.
    pub cookies: Option<CookieScope>,
    /// How much of the response body to preview, in bytes; `None` for none.
    pub body_preview: Option<usize>,
    /// Who is asking: origin, referrer and mixed-content handling.
    pub context: RequestContext,
}

impl Outbound {
    /// A plain GET with no body and no special handling.
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            method: "GET".into(),
            headers: Vec::new(),
            body: None,
            reach: AddressSpace::Loopback,
            streaming: false,
            cookies: None,
            body_preview: None,
            context: RequestContext::default(),
        }
    }
}

/// The broker's observer of one request in the network process. Events the
/// child reports pass through to `inner` - the engine's emitter for the
/// request - and whether a terminal one has passed is remembered, so the
/// request can be ended here when the child never ends it: cancelled before
/// anything went out, a child that died, a reply with no `Finished` behind it.
/// The embedder's request log hangs on a request that never ends.
pub struct ReportedRequest {
    inner: Arc<dyn NetObserver + Send + Sync>,
    url: String,
    started: std::time::Instant,
    done: AtomicBool,
    /// Longest body preview accepted from the child, in bytes.
    preview_cap: usize,
    /// Non-terminal events other than progress passed on so far; see
    /// [`MAX_EVENTS_PER_REQUEST`].
    events: std::sync::atomic::AtomicUsize,
    /// When progress was last passed on; see [`PROGRESS_INTERVAL`].
    last_progress: Mutex<Option<std::time::Instant>>,
    /// Whether the progress event that completes the body has been passed on.
    completed: AtomicBool,
}

/// Non-terminal events a request may report, progress aside. A redirect hop
/// costs a handful (resolved, connected, sent, redirected) and there are at
/// most twenty; past this the child is flooding, and the rest is dropped.
const MAX_EVENTS_PER_REQUEST: usize = 256;

/// Progress is passed on at most this often per request: a progress bar needs
/// no more, and a child sending it for every byte would otherwise fill the
/// embedder's control bus.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(50);

/// After a streamed body ends, how long the child's own terminal event has to
/// arrive over the link before the broker ends the request itself.
const STREAM_END_GRACE: Duration = Duration::from_secs(2);

impl ReportedRequest {
    fn new(inner: Arc<dyn NetObserver + Send + Sync>, url: String, preview_cap: usize) -> Self {
        Self {
            inner,
            url,
            started: std::time::Instant::now(),
            done: AtomicBool::new(false),
            preview_cap,
            events: std::sync::atomic::AtomicUsize::new(0),
            last_progress: Mutex::new(None),
            completed: AtomicBool::new(false),
        }
    }

    /// Whether the request has had its last event.
    pub fn is_done(&self) -> bool {
        self.done.load(Ordering::Relaxed)
    }

    /// An event the child reported. A terminal one claims the end first, so
    /// a cancel racing it on another thread finds the request already ended.
    ///
    /// Everything here came from the child, so it is held to what a real
    /// request produces: progress at most every [`PROGRESS_INTERVAL`] (and the
    /// completing one once), at most [`MAX_EVENTS_PER_REQUEST`] others, and a
    /// terminal event that cannot be read still ends the request, as failed.
    fn on_wire(&self, event: NetEventWire) {
        let terminal = event.is_terminal();
        if terminal {
            if self.done.swap(true, Ordering::Relaxed) {
                return;
            }
        } else if self.is_done() || !self.admit(&event) {
            return;
        }
        match event.into_net(self.preview_cap) {
            Some(event) => self.inner.on_event(event),
            // The end was claimed above; nothing else will report it.
            None if terminal => self.emit_failed("the network process reported an end the broker could not read"),
            None => {}
        }
    }

    /// Whether a non-terminal event is within the request's budget.
    fn admit(&self, event: &NetEventWire) -> bool {
        let NetEventWire::Progress {
            received_bytes,
            expected_length,
            ..
        } = event
        else {
            return self.events.fetch_add(1, Ordering::Relaxed) < MAX_EVENTS_PER_REQUEST;
        };
        if *expected_length == Some(*received_bytes) {
            return !self.completed.swap(true, Ordering::Relaxed);
        }
        let mut last = self.last_progress.lock();
        let now = std::time::Instant::now();
        if last.is_some_and(|at| now.duration_since(at) < PROGRESS_INTERVAL) {
            return false;
        }
        *last = Some(now);
        true
    }

    /// The streamed body is over, however it ended. The child's terminal event
    /// follows it over the link; if it has not come within `grace`, the
    /// request ends here - a child that never sends one would otherwise leave
    /// it open in the embedder's log for good.
    fn body_ended(&self, end: BodyEnd, grace: Duration) {
        std::thread::sleep(grace);
        if self.is_done() {
            return;
        }
        match end {
            BodyEnd::Complete(received_bytes) => {
                if self.done.swap(true, Ordering::Relaxed) {
                    return;
                }
                let Ok(url) = url::Url::parse(&self.url) else {
                    return;
                };
                self.inner.on_event(crate::net::events::NetEvent::Finished {
                    received_bytes,
                    elapsed: self.started.elapsed(),
                    url,
                });
            }
            BodyEnd::Failed(message) => self.fail(&message),
            BodyEnd::Abandoned => self.cancel(),
        }
    }

    /// The reply is in: end the request here if the child did not. A
    /// streamed head is not the end - the body is still arriving, and the
    /// child's `Finished` follows it over the link.
    fn finish(&self, outcome: &FetchOutcome) {
        let Ok(url) = url::Url::parse(&self.url) else {
            return;
        };
        match outcome {
            FetchOutcome::Streaming { .. } => {}
            FetchOutcome::Ok { body, .. } => {
                if !self.done.swap(true, Ordering::Relaxed) {
                    self.inner.on_event(crate::net::events::NetEvent::Finished {
                        received_bytes: body.len() as u64,
                        elapsed: self.started.elapsed(),
                        url,
                    });
                }
            }
            FetchOutcome::Error(message) => self.fail(message),
        }
    }

    /// End the request as failed, unless it has ended.
    fn fail(&self, message: &str) {
        if self.done.swap(true, Ordering::Relaxed) {
            return;
        }
        self.emit_failed(message);
    }

    /// Report the failure; the caller has claimed the end.
    fn emit_failed(&self, message: &str) {
        let Ok(url) = url::Url::parse(&self.url) else {
            return;
        };
        self.inner.on_event(crate::net::events::NetEvent::Failed {
            url,
            error: anyhow::Error::new(crate::engine::LoadError::Other {
                message: message.to_string(),
            }),
        });
    }

    /// End the request as cancelled, unless it has ended.
    fn cancel(&self) {
        if self.done.swap(true, Ordering::Relaxed) {
            return;
        }
        let Ok(url) = url::Url::parse(&self.url) else {
            return;
        };
        self.inner.on_event(crate::net::events::NetEvent::Cancelled {
            url,
            reason: "cancelled by the broker",
        });
    }
}

type Reported = Arc<Mutex<HashMap<RequestTag, Arc<ReportedRequest>>>>;

/// How a streamed body ended, as the broker saw it.
enum BodyEnd {
    Complete(u64),
    Failed(String),
    /// Nothing read the body to its end.
    Abandoned,
}

/// A streamed request still open when its reply arrived: what ends it once
/// the body has, if the child does not (see [`ReportedRequest::body_ended`]).
pub struct StreamEnd {
    request: Arc<ReportedRequest>,
    reported: Reported,
    tag: RequestTag,
}

impl StreamEnd {
    fn ended(self, end: BodyEnd) {
        self.request.body_ended(end, STREAM_END_GRACE);
        self.reported.lock().remove(&self.tag);
    }
}

impl std::fmt::Debug for StreamEnd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamEnd")
            .field("tag", &self.tag)
            .finish_non_exhaustive()
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
    /// For a streamed reply whose request is still open: ends it once the body
    /// has, should the child not.
    pub stream_end: Option<StreamEnd>,
}

impl NetReply {
    fn error(msg: impl Into<String>) -> Self {
        Self {
            outcome: FetchOutcome::Error(msg.into()),
            ring: None,
            stream_end: None,
        }
    }
}

#[cfg(target_os = "linux")]
fn recv_ring(rx: &mut gosub_ipc::EndpointRx) -> std::io::Result<RingFd> {
    rx.recv_fd()
}

/// The body behind a [`FromNet::SharedReply`]: its sealed memfd, read out whole.
#[cfg(target_os = "linux")]
fn recv_shared_body(rx: &mut gosub_ipc::EndpointRx, len: u64) -> std::io::Result<Vec<u8>> {
    let fd = rx.recv_fd()?;
    let len = usize::try_from(len).map_err(|_| std::io::Error::other("shared body length out of range"))?;
    gosub_ipc::shm::read_sealed_blob(fd, len)
}

#[cfg(not(target_os = "linux"))]
fn recv_shared_body(_rx: &mut gosub_ipc::EndpointRx, _len: u64) -> std::io::Result<Vec<u8>> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "no fd passing on this platform",
    ))
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

/// A network process that died is respawned at most this often.
const RESPAWN_COOLDOWN: Duration = Duration::from_secs(5);

/// How a respawned network process's vault line reaches the vault: the
/// vault's end of a fresh channel, whose other end the new process inherited.
pub type Relink = Box<dyn Fn(gosub_ipc::channel::Channel) + Send + Sync>;

/// Requests waiting for a reply, by tag.
type Pending = Arc<Mutex<HashMap<RequestTag, tokio::sync::oneshot::Sender<NetReply>>>>;

/// A running network process and the link to it. One that dies is respawned
/// on the next use (see [`NetProcess::ensure_alive`]); what was in flight on
/// it fails.
pub struct NetProcess {
    /// The broker's observer of each request that asked for one, by tag;
    /// kept until the request's last event, which for a streamed body comes
    /// after its reply.
    reported: Reported,
    tx: Arc<Mutex<EndpointTx>>,
    pending: Pending,
    next_tag: AtomicU64,
    child: Mutex<Option<gosub_sandbox::spawn::Child>>,
    /// Bounds concurrent requests (see [`MAX_INFLIGHT`]).
    inflight: Arc<tokio::sync::Semaphore>,
    /// The child holds a direct line to the cookie vault: requests may carry a
    /// cookie scope instead of a header. A respawned one is given a line too.
    vault_linked: bool,
    /// Who is waiting for an audit report, if anyone.
    audit_waiter: AuditWaiter,
    /// Cleared by the reader thread of the current link when it ends.
    alive: Mutex<Arc<AtomicBool>>,
    /// Serializes respawns and remembers the last attempt.
    respawn: Mutex<Option<std::time::Instant>>,
    /// Set by `shutdown`: no respawn after the engine let go.
    closed: AtomicBool,
    /// How a respawned process's vault line is handed to the vault.
    relink: Mutex<Option<Relink>>,
}

/// Audits in flight, by tag: each call waits for its own answer, and a late one
/// finds no waiter rather than the next caller's.
type AuditWaiter =
    Arc<Mutex<HashMap<RequestTag, std::sync::mpsc::SyncSender<Option<gosub_sandbox::audit::AuditReport>>>>>;

impl std::fmt::Debug for NetProcess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NetProcess").finish_non_exhaustive()
    }
}

/// A freshly spawned network process, before anything was sent to it.
struct Launched {
    tx: EndpointTx,
    rx: gosub_ipc::EndpointRx,
    child: gosub_sandbox::spawn::Child,
    vault_linked: bool,
}

/// Re-exec this binary as the network process; the link is connected,
/// nothing sent. With `vault`, the child also inherits its line to the vault.
fn launch(vault: Option<VaultLine>) -> anyhow::Result<Launched> {
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
    let (tx, rx) = endpoint.split();
    Ok(Launched {
        tx,
        rx,
        child,
        vault_linked: !extra_fds.is_empty(),
    })
}

/// Read the link until it ends: replies to their waiters, events to their
/// observers. At the end `alive` is cleared and everything still waiting is
/// woken, as failed.
fn start_reader(
    mut rx: gosub_ipc::EndpointRx,
    waiters: Pending,
    observers: Reported,
    audit_reply: AuditWaiter,
    ready_tx: std::sync::mpsc::SyncSender<()>,
    alive: Arc<AtomicBool>,
) -> std::io::Result<()> {
    // A plain thread, not a task: it blocks on the link, and must keep
    // draining even when every runtime worker is busy waiting on a reply.
    std::thread::Builder::new()
        .name("net-process-reader".into())
        .spawn(move || {
            while let Ok(msg) = rx.recv::<FromNet>() {
                match msg {
                    FromNet::Pong => {
                        let _ = ready_tx.send(());
                    }
                    FromNet::Audit { tag, report } => {
                        if let Some(waiter) = audit_reply.lock().remove(&tag) {
                            let _ = waiter.send(report);
                        }
                    }
                    // Only to an observer this side registered for the tag: a
                    // child cannot report on a request nobody asked it about.
                    FromNet::Event { tag, event } => {
                        let observer = observers.lock().get(&tag).cloned();
                        if let Some(observer) = observer {
                            observer.on_wire(event);
                            if observer.is_done() {
                                observers.lock().remove(&tag);
                            }
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
                                    stream_end: None,
                                },
                                Err(e) => NetReply::error(format!("body stream fd did not arrive: {e}")),
                            },
                            outcome => NetReply {
                                outcome,
                                ring: None,
                                stream_end: None,
                            },
                        };
                        if let Some(waiter) = waiters.lock().remove(&tag) {
                            let _ = waiter.send(reply);
                        }
                    }
                    // A body too large for a frame follows as a sealed memfd: read
                    // it now, before the next message, and hand on the plain `Ok`
                    // it stands for.
                    FromNet::SharedReply {
                        tag,
                        status,
                        status_text,
                        final_url,
                        headers,
                        peer_addr,
                        len,
                    } => {
                        let reply = match recv_shared_body(&mut rx, len) {
                            Ok(body) => NetReply {
                                outcome: FetchOutcome::Ok {
                                    status,
                                    status_text,
                                    final_url,
                                    headers,
                                    body,
                                    peer_addr,
                                },
                                ring: None,
                                stream_end: None,
                            },
                            Err(e) => NetReply::error(format!("shared body did not arrive: {e}")),
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
            audit_reply.lock().clear();
            // Whatever was still being reported on ends here, as failed:
            // its events died with the process.
            let abandoned: Vec<Arc<ReportedRequest>> = observers.lock().drain().map(|(_, r)| r).collect();
            for request in abandoned {
                request.fail("the network process went away");
            }
            // Only then marked dead: the maps are shared with the process a
            // respawn starts, and a request that finds this one dead respawns
            // and registers there. Cleared after that, it would be failed
            // with this process. One that reaches the dead link before this
            // store fails its send and removes itself.
            alive.store(false, Ordering::Release);
        })?;
    Ok(())
}

impl NetProcess {
    /// Re-exec this binary as the network process and connect to it. With
    /// `vault`, the child also inherits its line to the cookie vault.
    pub fn spawn(vault: Option<VaultLine>) -> anyhow::Result<Self> {
        let launched = launch(vault)?;
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let reported: Reported = Arc::new(Mutex::new(HashMap::new()));
        let audit_waiter: AuditWaiter = Arc::new(Mutex::new(HashMap::new()));
        let alive = Arc::new(AtomicBool::new(true));
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel::<()>(1);
        start_reader(
            launched.rx,
            Arc::clone(&pending),
            Arc::clone(&reported),
            Arc::clone(&audit_waiter),
            ready_tx,
            Arc::clone(&alive),
        )?;

        let net = Self {
            reported,
            tx: Arc::new(Mutex::new(launched.tx)),
            pending,
            next_tag: AtomicU64::new(1),
            child: Mutex::new(Some(launched.child)),
            inflight: Arc::new(tokio::sync::Semaphore::new(MAX_INFLIGHT)),
            vault_linked: launched.vault_linked,
            audit_waiter,
            alive: Mutex::new(alive),
            respawn: Mutex::new(None),
            closed: AtomicBool::new(false),
            relink: Mutex::new(None),
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

    /// The network process's pid, while it has one.
    pub fn pid(&self) -> Option<u32> {
        self.child.lock().as_ref().map(gosub_sandbox::spawn::Child::id)
    }

    /// Whether the current link is up.
    pub fn is_alive(&self) -> bool {
        self.alive.lock().load(Ordering::Acquire)
    }

    /// Register how a respawned process's vault line reaches the vault.
    pub fn on_relink(&self, relink: Relink) {
        *self.relink.lock() = Some(relink);
    }

    /// Respawn a dead network process: a new one, with a new line to the
    /// vault when this one had one, in place of the old. At most once per
    /// [`RESPAWN_COOLDOWN`]; until it is back, requests fail. Blocking, up to
    /// [`READY_TIMEOUT`]: call it off the runtime's workers.
    pub fn ensure_alive(&self) {
        if self.is_alive() || self.closed.load(Ordering::Acquire) {
            return;
        }
        let mut last = self.respawn.lock();
        if self.is_alive() || self.closed.load(Ordering::Acquire) {
            return;
        }
        if last.is_some_and(|at| at.elapsed() < RESPAWN_COOLDOWN) {
            return;
        }
        *last = Some(std::time::Instant::now());
        log::warn!("the network process died; respawning it");
        self.kill();

        // A new line for the vault, never none in place of one: without it
        // the vaulted requests would have no cookies, and the broker would
        // not attach them either.
        #[cfg(target_os = "linux")]
        let (vault_end, net_line) = if self.vault_linked {
            match gosub_ipc::channel::Channel::pair() {
                Ok((vault_end, net_end)) => (Some(vault_end), Some(VaultLine(net_end))),
                Err(e) => {
                    log::error!("no vault line for a new network process ({e}); requests fail");
                    return;
                }
            }
        } else {
            (None, None)
        };
        #[cfg(not(target_os = "linux"))]
        let (vault_end, net_line): (Option<gosub_ipc::channel::Channel>, Option<VaultLine>) = (None, None);

        let launched = match launch(net_line) {
            Ok(launched) => launched,
            Err(e) => {
                log::error!("the network process could not be respawned ({e}); requests fail");
                return;
            }
        };
        let alive = Arc::new(AtomicBool::new(true));
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel::<()>(1);
        if let Err(e) = start_reader(
            launched.rx,
            Arc::clone(&self.pending),
            Arc::clone(&self.reported),
            Arc::clone(&self.audit_waiter),
            ready_tx,
            Arc::clone(&alive),
        ) {
            log::error!("the network process reader could not start ({e}); requests fail");
            return;
        }
        let mut tx = launched.tx;
        *self.child.lock() = Some(launched.child);
        if tx.send(&ToNet::Ping).is_err() || ready_rx.recv_timeout(READY_TIMEOUT).is_err() {
            log::error!("the respawned network process did not answer; requests fail");
            self.kill();
            return;
        }
        *self.tx.lock() = tx;
        *self.alive.lock() = alive;

        // Handed over once the new process is in place. `respawn` and
        // `relink` are still held; a vault that respawns on the way relinks
        // through `relink_vault`, which takes neither.
        if let Some(vault_end) = vault_end {
            match self.relink.lock().as_ref() {
                Some(relink) => relink(vault_end),
                None => log::warn!("no way to hand the vault the new network line; requests go without cookies"),
            }
        }
        log::info!("the network process is back");
    }

    /// End the current child at once and reap it.
    fn kill(&self) {
        if let Some(mut child) = self.child.lock().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    /// Whether the child resolves cookies against the vault itself.
    pub fn vault_linked(&self) -> bool {
        self.vault_linked
    }

    /// The escape audit, run inside the network process. Blocking.
    pub fn audit(&self) -> anyhow::Result<Option<gosub_sandbox::audit::AuditReport>> {
        let tag = self.next_tag.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        self.audit_waiter.lock().insert(tag, tx);
        if let Err(e) = self.tx.lock().send(&ToNet::Audit { tag }) {
            self.audit_waiter.lock().remove(&tag);
            return Err(e.into());
        }
        let answer = rx.recv_timeout(Duration::from_secs(30));
        // Gone either way: answered, timed out, or woken by a closed link.
        self.audit_waiter.lock().remove(&tag);
        answer.map_err(|_| anyhow::anyhow!("the network process did not answer the audit"))
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
    ///
    /// `observer`, when given, hears what the network process reports about
    /// the request - the same events an in-process fetch would raise - and
    /// is guaranteed exactly one terminal event, from the child or from here.
    pub async fn fetch(
        &self,
        out: Outbound,
        cancel: &CancellationToken,
        observer: Option<Arc<dyn NetObserver + Send + Sync>>,
    ) -> NetReply {
        let tag = self.next_tag.fetch_add(1, Ordering::Relaxed);
        let reported = observer.map(|inner| {
            Arc::new(ReportedRequest::new(
                inner,
                out.url.clone(),
                out.body_preview.unwrap_or(0),
            ))
        });
        // Registered before anything goes out: the child's first event may
        // arrive before the send returns.
        if let Some(reported) = &reported {
            self.reported.lock().insert(tag, Arc::clone(reported));
        }
        let mut reply = self.fetch_tagged(tag, out, cancel).await;
        if let Some(reported) = reported {
            if cancel.is_cancelled() {
                reported.cancel();
            } else {
                reported.finish(&reply.outcome);
            }
            // A streamed body is still being reported on; the reader loop
            // lets go of it at its last event, or the body's end does.
            if reported.is_done() {
                self.reported.lock().remove(&tag);
            } else if reply.ring.is_some() {
                reply.stream_end = Some(StreamEnd {
                    request: reported,
                    reported: Arc::clone(&self.reported),
                    tag,
                });
            }
        }
        reply
    }

    async fn fetch_tagged(&self, tag: RequestTag, out: Outbound, cancel: &CancellationToken) -> NetReply {
        let Outbound {
            url,
            method,
            headers,
            body,
            reach,
            streaming,
            cookies,
            body_preview,
            context,
        } = out;
        let permit = tokio::select! {
            _ = cancel.cancelled() => return NetReply::error("cancelled"),
            p = self.inflight.clone().acquire_owned() => p,
        };
        let Ok(_permit) = permit else {
            return NetReply::error("the network process is shutting down");
        };

        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel::<NetReply>();
        self.pending.lock().insert(tag, reply_tx);
        let requested = url.clone();

        let msg = ToNet::Fetch(Box::new(NetFetch {
            tag,
            url,
            method,
            headers,
            body,
            reach,
            streaming,
            cookies,
            body_preview,
            context,
        }));
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
    ///
    /// Its size is checked too, since the frame cap is the only other bound: a
    /// failure's message and a status text are cut to [`MAX_EVENT_STRING`], and
    /// a reply past [`MAX_REPLY_HEADERS`], [`MAX_REPLY_HEADER_BYTES`] or
    /// [`MAX_REPLY_URL`] is refused.
    fn plausible_reply(mut reply: NetReply, requested: &str) -> NetReply {
        let (status_text, final_url, headers) = match &mut reply.outcome {
            FetchOutcome::Ok {
                status_text,
                final_url,
                headers,
                ..
            }
            | FetchOutcome::Streaming {
                status_text,
                final_url,
                headers,
                ..
            } => (status_text, final_url, headers),
            FetchOutcome::Error(message) => {
                *message = cut_string(std::mem::take(message), MAX_EVENT_STRING);
                return reply;
            }
        };
        *status_text = cut_string(std::mem::take(status_text), MAX_EVENT_STRING);
        let header_bytes: usize = headers.iter().map(|(n, v)| n.len() + v.len()).sum();
        if headers.len() > MAX_REPLY_HEADERS || header_bytes > MAX_REPLY_HEADER_BYTES {
            return NetReply::error(format!(
                "the network process sent {} headers ({header_bytes} bytes) for {requested}",
                headers.len()
            ));
        }
        if final_url.len() > MAX_REPLY_URL {
            return NetReply::error(format!(
                "the network process reported a {}-byte final url for {requested}",
                final_url.len()
            ));
        }
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
        // Under `respawn`: a respawn under way finishes first, and its child
        // is the one ended here; one that starts after sees `closed`.
        let _respawn = self.respawn.lock();
        self.closed.store(true, Ordering::Release);
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
    let stream_end = reply.stream_end;
    let (status, status_text, final_url, headers, peer_addr, body) = match reply.outcome {
        FetchOutcome::Ok {
            status,
            status_text,
            final_url,
            headers,
            body,
            peer_addr,
        } => (status, status_text, final_url, headers, peer_addr, Body::Whole(body)),
        FetchOutcome::Streaming {
            status,
            status_text,
            final_url,
            headers,
            peek,
            peer_addr,
        } => {
            let Some(ring) = reply.ring else {
                return Err(net_error("streamed reply without its ring"));
            };
            (
                status,
                status_text,
                final_url,
                headers,
                peer_addr,
                Body::Ring { peek, ring },
            )
        }
        FetchOutcome::Error(e) => return Err(net_error(e)),
    };

    let final_url = url::Url::parse(&final_url).map_err(|e| net_error(format!("bad final url: {e}")))?;

    let header_map = rebuild_headers(&headers);

    let content_type = header_map
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let content_length = header_map
        .get(http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok());

    let meta = |has_body: bool| {
        let mut meta = crate::net::types::FetchResultMeta::synthetic(final_url);
        meta.status = status;
        meta.status_text = status_text;
        meta.headers = header_map;
        meta.content_length = content_length;
        meta.content_type = content_type;
        meta.has_body = has_body;
        meta.peer_addr = peer_addr;
        meta
    };
    Ok(match body {
        Body::Whole(body) => crate::net::types::FetchResult::Buffered {
            meta: meta(!body.is_empty()),
            body: bytes::Bytes::from(body),
        },
        Body::Ring { peek, ring } => crate::net::types::FetchResult::Stream {
            meta: meta(true),
            peek_buf: gosub_sonar::types::PeekBuf::from_vec(peek),
            shared: drain_ring(ring, stream_end),
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
/// Chunks read off the ring and not yet taken by the body's pump. Bounded,
/// so a body nobody reads stalls the ring (and, past the ring's patience,
/// the network process ends the stream) rather than filling this process.
const RING_CHUNKS_AHEAD: usize = 8;
/// How long the body may go without a byte before it is ended: longer than
/// the network process's own read-idle timeout on the origin, so origin
/// silence is judged there, by the fetcher, with its configured bound.
const RING_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// Feed a [`SharedBody`] from a ring. The ring's reads block (bounded by its
/// stall timeout), so they run on a thread of their own; what it reads goes
/// through [`SharedBody::from_reader`], whose pump does not start consuming
/// until the first subscriber attaches - a `SharedBody` replays nothing to
/// a late subscriber, and the body's consumer attaches only after the
/// `Stream` result has crossed into the requester's task, by which time the
/// network process has long written the head of the body into the ring.
/// Pushing straight from the thread lost that head. The body ends when the
/// producer finishes; a producer that aborts or stalls ends it with an error.
/// Must run inside the I/O runtime, which the pump is spawned on.
///
/// [`SharedBody`]: gosub_sonar::net::shared_body::SharedBody
fn drain_ring(ring: RingFd, stream_end: Option<StreamEnd>) -> Arc<gosub_sonar::net::shared_body::SharedBody> {
    use gosub_sonar::net::shared_body::{ReaderOptions, SharedBody};
    let (chunks_tx, chunks_rx) = tokio::sync::mpsc::channel::<std::io::Result<bytes::Bytes>>(RING_CHUNKS_AHEAD);
    let spawned = std::thread::Builder::new()
        .name("net-ring-consumer".into())
        .spawn(move || {
            #[cfg(target_os = "linux")]
            let end = (|| {
                let mut consumer = match gosub_ipc::ring::RingConsumer::open(ring) {
                    Ok(c) => c,
                    Err(e) => {
                        let message = format!("body stream could not be opened: {e}");
                        let _ = chunks_tx.blocking_send(Err(std::io::Error::other(message.clone())));
                        return BodyEnd::Failed(message);
                    }
                };
                let mut buf = vec![0u8; 64 * 1024];
                let mut received = 0u64;
                loop {
                    match consumer.read(&mut buf) {
                        // EOF: dropping the sender ends the stream cleanly.
                        Ok(0) => return BodyEnd::Complete(received),
                        Ok(n) => {
                            received += n as u64;
                            if chunks_tx
                                .blocking_send(Ok(bytes::Bytes::copy_from_slice(&buf[..n])))
                                .is_err()
                            {
                                return BodyEnd::Abandoned; // the body was dropped unread
                            }
                        }
                        Err(e) => {
                            let message = format!("body stream failed: {e}");
                            let _ = chunks_tx.blocking_send(Err(std::io::Error::other(message.clone())));
                            return BodyEnd::Failed(message);
                        }
                    }
                }
            })();
            #[cfg(not(target_os = "linux"))]
            let end = {
                let _ = ring;
                let message = "body streams are not carried on this platform".to_string();
                let _ = chunks_tx.blocking_send(Err(std::io::Error::other(message.clone())));
                BodyEnd::Failed(message)
            };
            // The body is over for its reader; the request may still be open.
            drop(chunks_tx);
            if let Some(stream_end) = stream_end {
                stream_end.ended(end);
            }
        });
    if spawned.is_err() {
        let shared = Arc::new(SharedBody::new(RING_BODY_QUEUE));
        shared.error(net_error("could not start the body stream consumer"));
        return shared;
    }
    let chunks = Box::pin(futures::stream::unfold(chunks_rx, |mut rx| async move {
        rx.recv().await.map(|item| (item, rx))
    }));
    SharedBody::from_reader(
        tokio_util::io::StreamReader::new(chunks),
        ReaderOptions {
            capacity: RING_BODY_QUEUE,
            buf_size: 64 * 1024,
            cancel: None,
            idle_timeout: Some(RING_IDLE_TIMEOUT),
            total_timeout: None,
            // No cap: nothing here holds the body, and the in-process path has none.
            max_size: None,
            // What the in-process fetcher keeps for a subscriber that attaches late.
            replay_limit: gosub_sonar::net::shared_body::DEFAULT_REPLAY_LIMIT,
        },
    )
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use futures::StreamExt;

    /// The broker places a document by the address its response came from, so
    /// the network process's answer must carry it across the link; dropped, every
    /// private page in the isolated tier would count as public.
    #[test]
    fn the_peer_address_crosses_the_link() {
        let peer: std::net::SocketAddr = "127.0.0.1:8080".parse().unwrap();
        let wire = serde_json::to_vec(&FetchOutcome::Ok {
            status: 200,
            status_text: "OK".into(),
            final_url: "http://localhost:8080/".into(),
            headers: Vec::new(),
            body: b"hi".to_vec(),
            peer_addr: Some(peer),
        })
        .unwrap();
        let outcome: FetchOutcome = serde_json::from_slice(&wire).unwrap();
        let result = outcome_to_result(NetReply {
            outcome,
            ring: None,
            stream_end: None,
        })
        .unwrap();
        assert_eq!(result.meta().unwrap().peer_addr, Some(peer));
    }

    /// A reply's strings and headers are the child's: cut where cutting keeps
    /// the meaning, refused whole where it would not.
    #[test]
    fn a_reply_is_bounded_on_receipt() {
        let ok = |status_text: String, final_url: String, headers: HeaderList| NetReply {
            outcome: FetchOutcome::Ok {
                status: 200,
                status_text,
                final_url,
                headers,
                body: Vec::new(),
                peer_addr: None,
            },
            ring: None,
            stream_end: None,
        };
        let url = "https://site.test/".to_string();
        let failed = |reply: NetReply| match reply.outcome {
            FetchOutcome::Error(message) => message,
            other => panic!("not refused: {other:?}"),
        };

        let reply = NetProcess::plausible_reply(ok("x".repeat(MAX_EVENT_STRING * 2), url.clone(), Vec::new()), &url);
        match reply.outcome {
            FetchOutcome::Ok { status_text, .. } => assert_eq!(status_text.len(), MAX_EVENT_STRING),
            other => panic!("refused: {other:?}"),
        }

        let many: HeaderList = (0..=MAX_REPLY_HEADERS)
            .map(|i| (format!("x-{i}"), b"v".to_vec()))
            .collect();
        assert!(failed(NetProcess::plausible_reply(ok("OK".into(), url.clone(), many), &url)).contains("headers"));

        let heavy = vec![("x-big".to_string(), vec![b'v'; MAX_REPLY_HEADER_BYTES])];
        assert!(failed(NetProcess::plausible_reply(ok("OK".into(), url.clone(), heavy), &url)).contains("headers"));

        let long = format!("https://site.test/{}", "a".repeat(MAX_REPLY_URL));
        assert!(failed(NetProcess::plausible_reply(ok("OK".into(), long, Vec::new()), &url)).contains("final url"));

        let error = NetReply::error("e".repeat(MAX_EVENT_STRING * 2));
        assert_eq!(failed(NetProcess::plausible_reply(error, &url)).len(), MAX_EVENT_STRING);
    }

    /// What a request's observer was told, by kind.
    #[derive(Default)]
    struct Recorded(Mutex<Vec<&'static str>>);

    impl NetObserver for Recorded {
        fn on_event(&self, event: crate::net::events::NetEvent) {
            use crate::net::events::NetEvent;
            self.0.lock().push(match event {
                NetEvent::Finished { .. } => "finished",
                NetEvent::Failed { .. } => "failed",
                NetEvent::Cancelled { .. } => "cancelled",
                NetEvent::Progress { .. } => "progress",
                NetEvent::DnsResolved { .. } => "dns",
                _ => "other",
            });
        }
    }

    fn reported() -> (Arc<Recorded>, ReportedRequest) {
        let seen = Arc::new(Recorded::default());
        let request = ReportedRequest::new(seen.clone(), "https://site.test/".into(), 0);
        (seen, request)
    }

    /// A terminal event whose URL cannot be read still ends the request: it
    /// claims the end, so nothing else would.
    #[test]
    fn an_unreadable_end_still_ends_the_request() {
        let (seen, request) = reported();
        request.on_wire(NetEventWire::Finished {
            received_bytes: 1,
            elapsed_us: 1,
            url: "not a url".into(),
        });
        assert!(request.is_done());
        assert_eq!(*seen.0.lock(), vec!["failed"]);
    }

    /// A streamed body that ends without the child saying so is ended by the
    /// broker, the way the body ended.
    #[test]
    fn a_stream_without_a_terminal_event_is_ended_by_its_body() {
        let (seen, request) = reported();
        request.body_ended(BodyEnd::Complete(10), Duration::ZERO);
        assert_eq!(*seen.0.lock(), vec!["finished"]);

        let (seen, request) = reported();
        request.body_ended(BodyEnd::Failed("cut off".into()), Duration::ZERO);
        assert_eq!(*seen.0.lock(), vec!["failed"]);

        // The child's own end, when it comes in time, is the one reported.
        let (seen, request) = reported();
        request.on_wire(NetEventWire::Finished {
            received_bytes: 10,
            elapsed_us: 1,
            url: "https://site.test/".into(),
        });
        request.body_ended(BodyEnd::Abandoned, Duration::ZERO);
        assert_eq!(*seen.0.lock(), vec!["finished"]);
    }

    /// A child repeating events gets a request's worth through, not a flood:
    /// the completing progress once, other progress at most every interval,
    /// and at most a fixed number of anything else.
    #[test]
    fn a_flooding_child_is_held_to_a_requests_worth_of_events() {
        let (seen, request) = reported();
        for _ in 0..10_000 {
            request.on_wire(NetEventWire::Progress {
                received_bytes: 100,
                expected_length: Some(100),
                elapsed_us: 1,
            });
        }
        for i in 0..10_000u64 {
            request.on_wire(NetEventWire::Progress {
                received_bytes: i * 65_536,
                expected_length: None,
                elapsed_us: 1,
            });
        }
        for _ in 0..10_000 {
            request.on_wire(NetEventWire::DnsResolved {
                host: "site.test".into(),
                elapsed_us: 1,
                addr_count: 1,
            });
        }
        let seen = seen.0.lock();
        let count = |kind| seen.iter().filter(|k| **k == kind).count();
        assert_eq!(count("dns"), MAX_EVENTS_PER_REQUEST);
        // One completing, then whatever the interval let through while the
        // loop ran - a handful, not ten thousand.
        assert!(count("progress") < 50, "{} progress events", count("progress"));
    }

    /// The body's consumer attaches after the head has already crossed the
    /// ring; it must still see every byte.
    #[test]
    fn a_streamed_body_is_whole_for_a_late_subscriber() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let body: Vec<u8> = (0..300 * 1024).map(|i| (i % 253) as u8).collect();
        let (mut producer, fd) = gosub_ipc::ring::RingProducer::create(64 * 1024).unwrap();
        let expected = body.clone();
        // The ring is smaller than the body: the producer blocks until the
        // consumer drains, exactly as the network process would.
        let writer = std::thread::spawn(move || {
            producer.write_all(&body).unwrap();
            producer.finish();
        });
        let shared = {
            let _in_rt = rt.enter();
            drain_ring(fd, None)
        };
        // Late, as the real consumer is: the head of the body is in the ring,
        // and the consumer thread has had every chance to read it.
        std::thread::sleep(Duration::from_millis(200));
        let got = rt.block_on(async move {
            let mut stream = shared.subscribe_stream();
            let mut got = Vec::new();
            while let Some(chunk) = stream.next().await {
                got.extend_from_slice(&chunk.expect("a body chunk"));
            }
            got
        });
        writer.join().unwrap();
        assert_eq!(got.len(), expected.len(), "the late subscriber missed part of the body");
        assert_eq!(got, expected);
    }
}

/// A failure that came from (or about) the network process, as the engine's own
/// error type. `Other` because the cause is a broker↔child protocol problem
/// rather than any of the transport-specific variants.
pub fn net_error(msg: impl Into<String>) -> NetError {
    NetError::Other(std::sync::Arc::new(anyhow::anyhow!(msg.into())))
}
