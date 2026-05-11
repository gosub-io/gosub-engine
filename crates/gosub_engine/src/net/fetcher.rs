//! This module defines the `Fetcher` struct and its associated functionality for managing
//! and scheduling HTTP requests. It includes mechanisms for prioritizing requests, coalescing
//! identical requests, and handling streaming or buffered responses.

use crate::engine::types::{EventChannel, RequestId};
use crate::net::decision_hub::DecisionHub;
use crate::net::emitter::engine_event_emitter::EngineEventEmitter;
use crate::net::emitter::null_emitter::NullEmitter;
use crate::net::emitter::NetObserver;
use crate::net::fetch::{fetch_response_complete, fetch_response_top, ResponseTop};
use crate::net::pump::{spawn_pump, PumpCfg, PumpTargets};
use crate::net::req_ref_tracker::{RequestRefTracker, RequestReferenceMap};
use crate::net::shared_body::{ReaderOptions, SharedBody};
use crate::net::types::{FetchHandle, FetchKeyData, FetchRequest, FetchResult, NetError, Priority};
use crate::net::utils::{short_url, Waiter};
use crate::net::DecisionToken;
use crate::util::spawn_named;
use crate::Action;
use bytes::Bytes;
use dashmap::{DashMap, Entry};
use parking_lot::RwLock;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;
use std::{collections::VecDeque, sync::Arc, time::Duration};
use tokio::sync::{oneshot, Notify, Semaphore};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use url::Url;

/// How many shared consumers can listen for a resource
const SHARED_MAX_CAPACITY: usize = 32;

// Configuration for a fetcher
#[derive(Clone)]
pub struct FetcherConfig {
    /// Maximum number of concurrent fetches overall
    pub global_slots: usize,
    /// Maximum number of concurrent HTTP/1 connections per origin
    pub h1_per_origin: usize,
    /// Maximum number of concurrent HTTP/2 connections per origin
    pub h2_per_origin: usize,
    /// TCP connect timeout
    pub connect_timeout: Duration,
    /// Overall request timeout
    pub req_timeout: Duration,
    /// Max time between reads allowed
    pub read_idle_timeout: Duration,
    /// Max time for total body read
    pub total_body_timeout: Option<Duration>,
}

impl Default for FetcherConfig {
    fn default() -> Self {
        Self {
            global_slots: 32,
            h1_per_origin: 6,
            h2_per_origin: 16,
            connect_timeout: Duration::from_secs(5),
            req_timeout: Duration::from_secs(60),
            read_idle_timeout: Duration::from_secs(15),
            total_body_timeout: Some(Duration::from_secs(180)),
        }
    }
}

/// Represents an in-flight request, including its associated waiter and streaming preference.
pub struct FetchInflightEntry {
    /// Cancellation token for aborting the request
    parent_cancel: CancellationToken,
    /// Waiter for managing requests
    waiter: Arc<Waiter>,
    /// True when streaming is required
    wants_streaming: AtomicBool,
    /// Number of subscribers
    subs: AtomicUsize,
    /// Cancellation token that is triggered when all subscribers are gone
    done: CancellationToken,
}

impl FetchInflightEntry {
    #[inline]
    /// Increase the subscriber count
    fn inc_sub(&self) {
        self.subs.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    /// Decrease the subscriber count and cancel the parent if it was the last one
    fn dec_sub_and_maybe_cancel(&self) {
        // If this was the last subscriber, cancel the parent fetch
        if self.subs.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.parent_cancel.cancel();
        }
    }
}

/// Manages a map of in-flight fetch requests, allowing for coalescing of identical requests.
pub struct FetchInflightMap {
    /// Map of in-flight requests
    map: Arc<DashMap<FetchKeyData, Arc<FetchInflightEntry>>>,
    /// HTTP client for making requests
    client: Arc<reqwest::Client>,
    /// Observer to emit network events
    observer: Arc<dyn NetObserver + Send + Sync>,
    /// Configuration for the fetcher
    cfg: FetcherConfig,
}

impl FetchInflightMap {
    /// Creates a new `FetchInflightMap` with the given HTTP client, observer, and configuration.
    pub fn new(client: Arc<reqwest::Client>, observer: Arc<dyn NetObserver + Send + Sync>, cfg: FetcherConfig) -> Self {
        Self {
            map: Arc::new(DashMap::new()),
            client,
            observer,
            cfg,
        }
    }

    /// Join an existing inflight by key, or start the fetch task once.
    ///
    /// Returns `(handle, rx, was_new)`:
    /// - `handle`: per-caller child cancel token + req_id
    /// - `rx`: one-shot receiver for the final `FetchResult`
    /// - `was_new`: true if we spawned the fetch task
    pub fn join_or_start(
        &self,
        req: &FetchRequest,
        wants_stream: bool,
    ) -> (FetchHandle, oneshot::Receiver<FetchResult>, bool) {
        match self.map.entry(req.key_data.clone()) {
            Entry::Occupied(e) => {
                // Key already exists, join as waiter
                let entry = e.get().clone();

                let (tx, rx) = oneshot::channel();
                _ = entry.waiter.register(tx, wants_stream);

                let handle = FetchHandle {
                    req_id: RequestId::new(),
                    key: req.key_data.clone(),
                    cancel: entry.parent_cancel.child_token(),
                };
                (handle, rx, false)
            }
            Entry::Vacant(v) => {
                // No existing entry, create one and spawn fetch task
                let entry = Arc::new(FetchInflightEntry {
                    parent_cancel: CancellationToken::new(),
                    waiter: Arc::new(Waiter::new()),
                    wants_streaming: AtomicBool::new(wants_stream),
                    subs: AtomicUsize::new(0),
                    done: CancellationToken::new(),
                });

                let (tx, rx) = oneshot::channel();
                _ = entry.waiter.register(tx, wants_stream);

                v.insert(entry.clone());

                let key = req.key_data.clone();
                let map = self.map.clone();

                spawn_fetch_task(
                    req.clone(),
                    entry.clone(),
                    self.client.clone(),
                    self.observer.clone(),
                    self.cfg.clone(),
                    {
                        move || {
                            map.remove(&key);
                        }
                    },
                );

                let handle = FetchHandle {
                    req_id: RequestId::new(),
                    key: req.key_data.clone(),
                    cancel: entry.parent_cancel.child_token(),
                };

                (handle, rx, true)
            }
        }
    }
}

/// Represents an item in the fetch queue, including the request, its handle, and a channel to send the result back.
struct QueueItem {
    req: FetchRequest,
    handle: FetchHandle,
    reply: oneshot::Sender<FetchResult>,
}

/// The `Fetcher` struct manages the scheduling and execution of HTTP requests.
/// It supports prioritization, coalescing of identical requests, and streaming or buffered responses.
pub struct Fetcher {
    /// HTTP client for making requests
    client: reqwest::Client,
    /// Configuration for the fetcher
    cfg: FetcherConfig,

    /// Semaphore for limiting global current fetches
    global_slots: Arc<Semaphore>,
    /// Map for managing per-origin limits
    per_origin: DashMap<String, Arc<Semaphore>>,

    /// Queue for high priority requests
    q_high: tokio::sync::Mutex<VecDeque<QueueItem>>,
    /// Queue for regular priority request
    q_norm: tokio::sync::Mutex<VecDeque<QueueItem>>,
    /// Queue for low priority requests
    q_low: tokio::sync::Mutex<VecDeque<QueueItem>>,
    /// Queue for idle priority requests
    q_idle: tokio::sync::Mutex<VecDeque<QueueItem>>,

    /// Map for managing inflight requests and their associated waiters
    inflight_map: Arc<DashMap<String, Arc<FetchInflightEntry>>>,

    /// Notifier to wake up the fetcher when a new request is submitted
    wake: Notify,

    /// Event channel to emit engine events
    event_tx: EventChannel,

    // /// Io channel to send IO commands (fetching sub-resources, etc.)
    // io_tx: IoChannel,
    /// Decision hub for handling user decisions on requests
    decision_hub: Arc<DecisionHub>,

    /// Map to track request references for associating requests with their tabs
    request_reference_map: Arc<RwLock<RequestReferenceMap>>,
    request_reference_tracker: Arc<RequestRefTracker>,
}

impl Fetcher {
    /// Creates a new `Fetcher` instance with the given configuration
    pub fn new(
        config: FetcherConfig,
        event_tx: EventChannel,
        request_reference_map: Arc<RwLock<RequestReferenceMap>>,
    ) -> Self {
        // Start default client
        let client = reqwest::Client::builder()
            .connection_verbose(true)
            .http2_adaptive_window(true)
            .connect_timeout(config.connect_timeout)
            .timeout(config.req_timeout)
            .use_rustls_tls()
            .build()
            .expect("reqwest client build failed");

        Self {
            client,
            cfg: config.clone(),
            global_slots: Arc::new(Semaphore::new(config.global_slots)),
            per_origin: DashMap::new(),
            q_high: tokio::sync::Mutex::new(VecDeque::new()),
            q_norm: tokio::sync::Mutex::new(VecDeque::new()),
            q_low: tokio::sync::Mutex::new(VecDeque::new()),
            q_idle: tokio::sync::Mutex::new(VecDeque::new()),
            inflight_map: Arc::new(DashMap::new()),
            wake: Notify::new(),
            event_tx,
            // io_tx,
            decision_hub: Arc::new(DecisionHub::new()),
            request_reference_map: request_reference_map.clone(),
            request_reference_tracker: Arc::new(RequestRefTracker::new()),
        }
    }

    /// Returns the origin of a URL as a string key.
    fn origin_key(url: &Url) -> String {
        // Use origin (scheme + host + port) as key
        url.origin().ascii_serialization()
    }

    /// Pick the next request to process, using weighted round-robin across priority lanes.
    /// There are probably better schedulers, but this is simple and effective.
    fn pick_lane<'a>(
        &'a self,
        high: &'a mut VecDeque<QueueItem>,
        norm: &'a mut VecDeque<QueueItem>,
        low: &'a mut VecDeque<QueueItem>,
        idle: &'a mut VecDeque<QueueItem>,
        counter: &mut u8,
    ) -> Option<QueueItem> {
        // Weighted round-robin: 8:4:2:1

        let slot = *counter as usize;
        *counter = (*counter + 1) % 15; // 8 + 4 + 2 + 1 = 15 slots

        let try_pop = |q: &mut VecDeque<QueueItem>| q.pop_front();

        match slot {
            0..=7 => try_pop(high)
                .or_else(|| try_pop(norm))
                .or_else(|| try_pop(low))
                .or_else(|| try_pop(idle)),
            8..=11 => try_pop(norm)
                .or_else(|| try_pop(high))
                .or_else(|| try_pop(low))
                .or_else(|| try_pop(idle)),
            12..=13 => try_pop(low)
                .or_else(|| try_pop(norm))
                .or_else(|| try_pop(high))
                .or_else(|| try_pop(idle)),
            _ => try_pop(idle)
                .or_else(|| try_pop(low))
                .or_else(|| try_pop(norm))
                .or_else(|| try_pop(high)),
        }
    }

    /// Submit a fetch request to the appropriate priority lane.
    pub async fn submit(&self, req: FetchRequest, req_handle: FetchHandle, reply_tx: oneshot::Sender<FetchResult>) {
        log::debug!("Submitting fetch request: {:?}", req);

        let mut lane = match req.priority {
            Priority::High => self.q_high.lock().await,
            Priority::Normal => self.q_norm.lock().await,
            Priority::Low => self.q_low.lock().await,
            Priority::Idle => self.q_idle.lock().await,
        };
        lane.push_back(QueueItem {
            req,
            handle: req_handle,
            reply: reply_tx,
        });
        self.wake.notify_one();
    }

    /// Runs the fetcher, processing requests from the priority queues
    pub async fn run(&self, mut shutdown_tx: tokio::sync::watch::Receiver<bool>) {
        let mut lane_counter: u8 = 0;

        loop {
            // Check for shutdown
            if *shutdown_tx.borrow() {
                break;
            }

            // Pull next request (if any)
            let next = {
                let mut high = self.q_high.lock().await;
                let mut norm = self.q_norm.lock().await;
                let mut low = self.q_low.lock().await;
                let mut idle = self.q_idle.lock().await;
                self.pick_lane(&mut high, &mut norm, &mut low, &mut idle, &mut lane_counter)
            };

            // If none, wait for notification of new requests, or shutdown
            let Some(QueueItem {
                req,
                handle,
                reply: reply_tx,
            }) = next
            else {
                tokio::select! {
                    _ = self.wake.notified() => {},
                    _ = shutdown_tx.changed() => {},
                }
                continue;
            };

            // Coalescing: if an identical request is in-flight, register and move on so we don't duplicate work
            let key_opt = req.key_data.generate();
            let key_str = match key_opt {
                Some(key_str) => {
                    // Found a key we can use for coalescing
                    key_str
                }
                None => {
                    // No key found we can use for coalescing. This can happen if we have non-safe requests like POST, PUT etc.
                    // We must generate a unique one so this request is not coalesced with anything else.
                    format!(
                        "{} {} @{}",
                        req.key_data.method,
                        req.key_data.url,
                        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
                    )
                }
            };

            // Get or create the inflight entry
            let (inflight_entry, is_leader) = match self.inflight_map.entry(key_str.clone()) {
                Entry::Occupied(entry) => (entry.get().clone(), false),
                Entry::Vacant(v) => {
                    let arc = Arc::new(FetchInflightEntry {
                        parent_cancel: CancellationToken::new(),
                        waiter: Arc::new(Waiter::new()),
                        wants_streaming: AtomicBool::new(req.streaming),
                        done: CancellationToken::new(),
                        subs: AtomicUsize::new(0),
                    });
                    v.insert(arc.clone());
                    (arc, true)
                }
            };

            // If we are the leader (if we've just created the entry), we need to track the request reference
            if is_leader {
                self.request_reference_tracker.inc(&req.reference);
            }

            // Register the caller to the waiter
            inflight_entry.waiter.register(reply_tx, req.streaming).await;

            // Increase ref counter
            inflight_entry.inc_sub();

            // If this caller is cancelled, we decrease the ref count and maybe cancel the parent
            let child_cancel = handle.cancel.clone();
            let entry_for_cancel = inflight_entry.clone();
            let done = entry_for_cancel.done.clone();
            tokio::spawn(async move {
                tokio::select! {
                    _ = child_cancel.cancelled() => entry_for_cancel.dec_sub_and_maybe_cancel(),
                    _ = done.cancelled() => {}
                }
            });

            // Update streaming preference if needed
            if req.streaming {
                inflight_entry.wants_streaming.store(true, Ordering::Relaxed);
            }

            // Followers are done; leader will spawn the fetch task
            if !is_leader {
                continue;
            }

            // Setup the observer to emit events to the UA
            let observer: Arc<dyn NetObserver + Send + Sync> = {
                let guard = self.request_reference_map.read();
                match guard.get(&req.reference) {
                    Some(tab_id) => Arc::new(EngineEventEmitter::new(
                        *tab_id, // here is where we connect "events" to tabs
                        req.req_id,
                        req.reference,
                        self.event_tx.clone(),
                        req.kind,
                        req.initiator,
                    )),
                    None => {
                        log::trace!(
                            "Cannot find the request reference for req_id {:?} reference {:?}",
                            req.req_id,
                            req.reference
                        );
                        Arc::new(NullEmitter) as Arc<dyn NetObserver + Send + Sync>
                    }
                }
            };

            let client = self.client.clone();
            let global = self.global_slots.clone();
            let per_origin = self.per_origin.clone();
            let cfg = self.cfg.clone();
            let inflight = self.inflight_map.clone();
            let key_for_remove = key_str.clone();
            let inflight_entry2 = inflight_entry.clone();
            let mut shutdown_child = shutdown_tx.clone();
            let req_ref_map_clone = self.request_reference_map.clone();
            let req_for_task = req.clone();
            let cancel_parent = inflight_entry2.parent_cancel.clone();
            let ref_ref_tracker_clone = self.request_reference_tracker.clone();

            let title = format!("Fetcher: {}", short_url(&req.key_data.url, 80));
            spawn_named(&title, async move {
                let origin = Fetcher::origin_key(&req.key_data.url);
                let slots = per_origin
                    .entry(origin.clone())
                    .or_insert_with(|| Arc::new(Semaphore::new(per_origin_limit_for(&cfg, &req.key_data.url))))
                    .clone();

                let g = tokio::select! { p = global.acquire_owned() => Some(p), _ = shutdown_child.changed() => None };
                if g.is_none() {
                    return;
                } // guard will drop -> cleanup

                let h = tokio::select! { p = slots.acquire_owned() => Some(p), _ = shutdown_child.changed() => None };
                if h.is_none() {
                    return;
                } // guard will drop -> cleanup

                let should_stream = req.streaming || inflight_entry2.wants_streaming.load(Ordering::Relaxed);

                // Perform the request, either streaming or buffered
                let result = if should_stream {
                    perform_streaming(&client, observer.clone(), &req_for_task, &cfg, cancel_parent.clone()).await
                } else {
                    perform_buffered(&client, observer.clone(), &req_for_task, &cfg, cancel_parent.clone()).await
                };

                // If we found an error, convert it to a FetchResult::Error
                let fr = match &result {
                    Ok(fetch_result) => fetch_result.clone(),
                    Err(e) => FetchResult::Error(e.clone()),
                };

                // Fanout the fetchresult (either the stream or the buffer) all listeners. Note that
                // the waiter deals with streaming vs buffered listeners internally.
                inflight_entry2.waiter.finish(fr).await;

                // Cleanup
                inflight_entry2.done.cancel();
                inflight.remove(&key_for_remove);

                // Decrease the request reference count and maybe clean it up when nothing is using it anymore
                ref_ref_tracker_clone.dec_and_maybe_cleanup(&req.reference, &req_ref_map_clone);
            });
        }
    }

    /// Fulfill the decision for a given token with the specified action.
    pub async fn fulfill(&self, token: DecisionToken, action: Action) {
        self.decision_hub.fulfill(token, action);
    }
}

// Choose per-origin limit based on scheme/alpn (rough heuristic here).
fn per_origin_limit_for(cfg: &FetcherConfig, url: &Url) -> usize {
    match url.scheme() {
        // reqwest will use h2 when it can; safe cap
        "http" | "https" => cfg.h2_per_origin,
        _ => cfg.h1_per_origin,
    }
}

/// Perform the actual HTTP request using reqwest.
async fn perform_streaming(
    client: &reqwest::Client,
    observer: Arc<dyn NetObserver + Send + Sync>,
    req: &FetchRequest,
    cfg: &FetcherConfig,
    cancel: CancellationToken,
) -> Result<FetchResult, NetError> {
    // Get the response top (headers + peek)
    let ResponseTop { meta, peek_buf, reader } = fetch_response_top(
        Arc::new(client.clone()),
        req.key_data.url.clone(),
        cancel.clone(),
        observer.clone(),
    )
    .await?;

    let opts = ReaderOptions {
        capacity: SHARED_MAX_CAPACITY,
        buf_size: 16 * 1024,
        cancel: Some(cancel.clone()),
        idle_timeout: Some(cfg.read_idle_timeout),
        total_timeout: cfg.total_body_timeout,
        max_size: None,
    };

    Ok(FetchResult::Stream {
        meta,
        peek_buf,
        shared: SharedBody::from_reader(reader, opts),
    })
}

/// Perform an HTTP request using buffered mode
async fn perform_buffered(
    // Reqwest client
    client: &reqwest::Client,
    // Observer to emit NetEvents to
    observer: Arc<dyn NetObserver + Send + Sync>,
    // Actual request
    req: &FetchRequest,
    // Config
    cfg: &FetcherConfig,
    // Cancellation token
    cancel: CancellationToken,
) -> Result<FetchResult, NetError> {
    let (meta, body) = fetch_response_complete(
        Arc::new(client.clone()),
        req.key_data.url.clone(),
        cancel.clone(),
        observer,
        req.max_bytes,
        cfg.read_idle_timeout,
        cfg.total_body_timeout,
    )
    .await?;

    Ok(FetchResult::Buffered {
        meta,
        body: Bytes::from(body),
    })
}

pub fn spawn_fetch_task(
    req: FetchRequest,
    entry: Arc<FetchInflightEntry>,
    client: Arc<reqwest::Client>,
    observer: Arc<dyn NetObserver + Send + Sync>,
    cfg: FetcherConfig,
    on_finish: impl FnOnce() + Send + 'static,
) -> JoinHandle<()> {
    let url = req.key_data.url.clone();
    let cancel_parent = entry.parent_cancel.clone();

    spawn_named(&format!("Fetch: {}", short_url(&url, 80)), async move {
        // Make sure we always do a cleanup
        struct Cleanup<F: FnOnce()>(Option<F>);
        impl<F: FnOnce()> Drop for Cleanup<F> {
            fn drop(&mut self) {
                if let Some(f) = self.0.take() {
                    f();
                }
            }
        }
        let _cleanup = Cleanup(Some(on_finish));

        let top = match fetch_response_top(client.clone(), url.clone(), cancel_parent.clone(), observer.clone()).await {
            Ok(top) => top,
            Err(e) => {
                let _ = entry.waiter.finish(FetchResult::Error(e)).await;
                return;
            }
        };
        let ResponseTop { meta, peek_buf, reader } = top;

        // This is the shared body into which we pump the reqwest reader
        let shared = Arc::new(SharedBody::new(SHARED_MAX_CAPACITY));

        let pump_cfg = PumpCfg {
            idle: cfg.read_idle_timeout,
            total_deadline: cfg.total_body_timeout.map(|d| Instant::now() + d),
        };

        // We "pump" the reqwest reader into the shared body. In this instance, we just
        // pass from one reader to another one (no transformation), but the pump can only be
        // used to send data to disk, or even to both disk and a shared body.
        let _pump = spawn_pump(
            reader,
            PumpTargets {
                shared: Some(shared.clone()),
                file_dest: None,
                peek_buf: peek_buf.clone(),
            },
            pump_cfg,
            cancel_parent.clone(),
            observer.clone(),
            url.clone(),
        );

        let res = FetchResult::Stream { meta, peek_buf, shared };
        let _ = entry.waiter.finish(res).await;

        // cleanup will be called here
    })
}
