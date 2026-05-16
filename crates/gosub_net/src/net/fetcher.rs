use crate::net::decision_hub::DecisionHub;
use crate::net::fetch::{fetch_response_complete, fetch_response_top, ResponseTop};
use crate::net::fetcher_context::FetcherContext;
use crate::net::observer::NetObserver;
use crate::net::pump::{spawn_pump, PumpCfg, PumpTargets};
use crate::net::shared_body::{ReaderOptions, SharedBody};
use crate::net::types::{FetchHandle, FetchKeyData, FetchRequest, FetchResult, NetError, Priority};
use crate::net::utils::{short_url, spawn_named, Waiter};
use crate::types::{Action, DecisionToken, RequestId};
use bytes::Bytes;
use dashmap::{DashMap, Entry};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;
use std::{collections::VecDeque, sync::Arc, time::Duration};
use tokio::sync::{oneshot, Notify, Semaphore};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use url::Url;

const SHARED_MAX_CAPACITY: usize = 32;

/// Configuration for the priority-scheduled [`Fetcher`].
///
/// All timeouts apply per individual request, not to the fetcher as a whole.
/// The default values are conservative browser-like settings suitable for
/// general-purpose use; tune them for your environment.
#[derive(Clone)]
pub struct FetcherConfig {
    /// Maximum number of concurrent HTTP connections across all origins.
    pub global_slots: usize,
    /// Maximum concurrent connections **per origin** for HTTP/1.x.
    /// HTTP/1 pipelines poorly, so browsers cap this at 6.
    pub h1_per_origin: usize,
    /// Maximum concurrent streams **per origin** for HTTP/2 (multiplexed).
    pub h2_per_origin: usize,
    /// Timeout for the TCP + TLS handshake.  Applies before any bytes are sent.
    pub connect_timeout: Duration,
    /// Timeout from sending the first request byte until the response headers arrive.
    pub req_timeout: Duration,
    /// Maximum silence between consecutive body chunks before the read is aborted.
    pub read_idle_timeout: Duration,
    /// Wall-clock deadline for receiving the entire response body after headers.
    /// `None` disables the deadline (useful for very large downloads).
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

pub struct FetchInflightEntry {
    parent_cancel: CancellationToken,
    waiter: Arc<Waiter>,
    wants_streaming: AtomicBool,
    subs: AtomicUsize,
    done: CancellationToken,
}

impl FetchInflightEntry {
    #[inline]
    fn inc_sub(&self) {
        self.subs.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    fn dec_sub_and_maybe_cancel(&self) {
        if self.subs.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.parent_cancel.cancel();
        }
    }
}

pub struct FetchInflightMap {
    map: Arc<DashMap<FetchKeyData, Arc<FetchInflightEntry>>>,
    client: Arc<reqwest::Client>,
    observer: Arc<dyn NetObserver + Send + Sync>,
    cfg: FetcherConfig,
}

impl FetchInflightMap {
    pub fn new(client: Arc<reqwest::Client>, observer: Arc<dyn NetObserver + Send + Sync>, cfg: FetcherConfig) -> Self {
        Self {
            map: Arc::new(DashMap::new()),
            client,
            observer,
            cfg,
        }
    }

    pub fn join_or_start(
        &self,
        req: &FetchRequest,
        wants_stream: bool,
    ) -> (FetchHandle, oneshot::Receiver<FetchResult>, bool) {
        match self.map.entry(req.key_data.clone()) {
            Entry::Occupied(e) => {
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
                    move || {
                        map.remove(&key);
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

struct QueueItem {
    req: FetchRequest,
    handle: FetchHandle,
    reply: oneshot::Sender<FetchResult>,
}

pub struct Fetcher {
    client: reqwest::Client,
    cfg: FetcherConfig,

    global_slots: Arc<Semaphore>,
    per_origin: DashMap<String, Arc<Semaphore>>,

    q_high: tokio::sync::Mutex<VecDeque<QueueItem>>,
    q_norm: tokio::sync::Mutex<VecDeque<QueueItem>>,
    q_low: tokio::sync::Mutex<VecDeque<QueueItem>>,
    q_idle: tokio::sync::Mutex<VecDeque<QueueItem>>,

    inflight_map: Arc<DashMap<String, Arc<FetchInflightEntry>>>,

    wake: Notify,

    decision_hub: Arc<DecisionHub>,

    ctx: Arc<dyn FetcherContext>,
}

impl Fetcher {
    pub fn new(config: FetcherConfig, ctx: Arc<dyn FetcherContext>) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .connection_verbose(true)
            .http2_adaptive_window(true)
            .connect_timeout(config.connect_timeout)
            .timeout(config.req_timeout)
            .use_rustls_tls()
            .build()?;

        Ok(Self {
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
            decision_hub: Arc::new(DecisionHub::new()),
            ctx,
        })
    }

    fn origin_key(url: &Url) -> String {
        url.origin().ascii_serialization()
    }

    // Weighted round-robin dequeue across the four priority lanes.
    // The 15-slot cycle gives approximate weights: High=8, Normal=4, Low=2, Idle=1.
    // When the preferred lane is empty the next non-empty lane is tried in
    // descending priority order, so no request starves as long as slots remain.
    fn pick_lane<'a>(
        &'a self,
        high: &'a mut VecDeque<QueueItem>,
        norm: &'a mut VecDeque<QueueItem>,
        low: &'a mut VecDeque<QueueItem>,
        idle: &'a mut VecDeque<QueueItem>,
        counter: &mut u8,
    ) -> Option<QueueItem> {
        let slot = *counter as usize;
        *counter = (*counter + 1) % 15;

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

    pub async fn run(&self, mut shutdown_tx: tokio::sync::watch::Receiver<bool>) {
        let mut lane_counter: u8 = 0;

        loop {
            if *shutdown_tx.borrow() {
                break;
            }

            let next = {
                let mut high = self.q_high.lock().await;
                let mut norm = self.q_norm.lock().await;
                let mut low = self.q_low.lock().await;
                let mut idle = self.q_idle.lock().await;
                self.pick_lane(&mut high, &mut norm, &mut low, &mut idle, &mut lane_counter)
            };

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

            let key_opt = req.key_data.generate();
            let key_str = match key_opt {
                Some(key_str) => key_str,
                None => format!(
                    "{} {} @{}",
                    req.key_data.method,
                    req.key_data.url,
                    chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
                ),
            };

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

            if is_leader {
                self.ctx.on_ref_active(req.reference);
            }

            inflight_entry.waiter.register(reply_tx, req.streaming).await;
            inflight_entry.inc_sub();

            let child_cancel = handle.cancel.clone();
            let entry_for_cancel = inflight_entry.clone();
            let done = entry_for_cancel.done.clone();
            tokio::spawn(async move {
                tokio::select! {
                    _ = child_cancel.cancelled() => entry_for_cancel.dec_sub_and_maybe_cancel(),
                    _ = done.cancelled() => {}
                }
            });

            if req.streaming {
                inflight_entry.wants_streaming.store(true, Ordering::Relaxed);
            }

            if !is_leader {
                continue;
            }

            let observer = self
                .ctx
                .observer_for(req.reference, req.req_id, req.kind, req.initiator);

            let client = self.client.clone();
            let global = self.global_slots.clone();
            let per_origin = self.per_origin.clone();
            let cfg = self.cfg.clone();
            let inflight = self.inflight_map.clone();
            let key_for_remove = key_str.clone();
            let inflight_entry2 = inflight_entry.clone();
            let mut shutdown_child = shutdown_tx.clone();
            let req_for_task = req.clone();
            let cancel_parent = inflight_entry2.parent_cancel.clone();
            let ctx_clone = self.ctx.clone();

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
                }

                let h = tokio::select! { p = slots.acquire_owned() => Some(p), _ = shutdown_child.changed() => None };
                if h.is_none() {
                    return;
                }

                let should_stream = req.streaming || inflight_entry2.wants_streaming.load(Ordering::Relaxed);

                let result = if should_stream {
                    perform_streaming(&client, observer.clone(), &req_for_task, &cfg, cancel_parent.clone()).await
                } else {
                    perform_buffered(&client, observer.clone(), &req_for_task, &cfg, cancel_parent.clone()).await
                };

                let fr = match &result {
                    Ok(fetch_result) => fetch_result.clone(),
                    Err(e) => FetchResult::Error(e.clone()),
                };

                inflight_entry2.waiter.finish(fr).await;

                inflight_entry2.done.cancel();
                inflight.remove(&key_for_remove);

                ctx_clone.on_ref_done(req.reference);
            });
        }
    }

    pub async fn fulfill(&self, token: DecisionToken, action: Action) {
        self.decision_hub.fulfill(token, action);
    }
}

fn per_origin_limit_for(cfg: &FetcherConfig, url: &Url) -> usize {
    match url.scheme() {
        "http" | "https" => cfg.h2_per_origin,
        _ => cfg.h1_per_origin,
    }
}

async fn perform_streaming(
    client: &reqwest::Client,
    observer: Arc<dyn NetObserver + Send + Sync>,
    req: &FetchRequest,
    cfg: &FetcherConfig,
    cancel: CancellationToken,
) -> Result<FetchResult, NetError> {
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

async fn perform_buffered(
    client: &reqwest::Client,
    observer: Arc<dyn NetObserver + Send + Sync>,
    req: &FetchRequest,
    cfg: &FetcherConfig,
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

        let shared = Arc::new(SharedBody::new(SHARED_MAX_CAPACITY));

        let pump_cfg = PumpCfg {
            idle: cfg.read_idle_timeout,
            total_deadline: cfg.total_body_timeout.map(|d| Instant::now() + d),
        };

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
    })
}
