//! # `gosub_net` network stack
//!
//! Async HTTP/HTTPS fetching for the Gosub engine.  Two APIs are provided —
//! choose the one that matches your caller's complexity:
//!
//! ## Quick reference
//!
//! | Caller | API | When to use |
//! |---|---|---|
//! | Renderer, tools, one-shot scripts | [`simple::simple_get`] | No scheduler needed; just give me bytes |
//! | Engine, tabs, anything with priority / coalescing | [`fetcher::Fetcher`] | Full scheduler |
//!
//! ---
//!
//! ## `simple_get` — fire and forget
//!
//! ```rust,ignore
//! use gosub_net::net::simple::simple_get;
//! use url::Url;
//!
//! let bytes = simple_get(&Url::parse("https://example.org").unwrap()).await?;
//! ```
//!
//! Supports `http://`, `https://`, and `file://`.  Returns `anyhow::Result<Bytes>`.
//! Creates a fresh reqwest client per call, so it is not suitable for high-volume use.
//!
//! ---
//!
//! ## `Fetcher` — priority scheduler
//!
//! The full scheduler deduplicates identical in-flight requests, enforces per-origin
//! and global concurrency limits, and fans out results to multiple subscribers.
//!
//! ### Minimal standalone setup
//!
//! ```rust,ignore
//! use std::sync::Arc;
//! use gosub_net::net::fetcher::{Fetcher, FetcherConfig};
//! use gosub_net::net::fetcher_context::FetcherContext;
//! use gosub_net::net::null_emitter::NullEmitter;
//! use gosub_net::net::observer::NetObserver;
//! use gosub_net::net::request_ref::RequestReference;
//! use gosub_net::net::types::{FetchHandle, FetchKeyData, FetchRequest, FetchResult, Initiator, Priority, ResourceKind};
//! use gosub_net::types::RequestId;
//! use tokio::sync::{oneshot, watch};
//! use tokio_util::sync::CancellationToken;
//! use url::Url;
//!
//! struct MyContext;
//! impl FetcherContext for MyContext {
//!     fn observer_for(&self, _ref: RequestReference, _id: RequestId, _kind: ResourceKind, _init: Initiator)
//!         -> Arc<dyn NetObserver + Send + Sync> { Arc::new(NullEmitter) }
//!     fn on_ref_active(&self, _: RequestReference) {}
//!     fn on_ref_done(&self, _: RequestReference) {}
//! }
//!
//! let (shutdown_tx, shutdown_rx) = watch::channel(false);
//! let fetcher = Arc::new(Fetcher::new(FetcherConfig::default(), Arc::new(MyContext)));
//!
//! // Spawn the scheduler loop — it runs until shutdown_rx becomes true.
//! let f = fetcher.clone();
//! tokio::spawn(async move { f.run(shutdown_rx).await });
//!
//! // Build and submit a request.
//! let url = Url::parse("https://example.org").unwrap();
//! let key = FetchKeyData::new(url);
//! let req_id = RequestId::new();
//! let req = FetchRequest {
//!     reference: RequestReference::Background(0),
//!     req_id,
//!     key_data: key.clone(),
//!     priority: Priority::Normal,
//!     initiator: Initiator::Other,
//!     kind: ResourceKind::Document,
//!     streaming: false,
//!     auto_decode: true,
//!     max_bytes: None,
//! };
//! let handle = FetchHandle { req_id, key, cancel: CancellationToken::new() };
//! let (tx, rx) = oneshot::channel();
//! fetcher.submit(req, handle, tx).await;
//!
//! match rx.await.unwrap() {
//!     FetchResult::Buffered { meta, body } => println!("HTTP {} — {} bytes", meta.status, body.len()),
//!     FetchResult::Stream { .. } => println!("streaming response"),
//!     FetchResult::Error(e) => eprintln!("error: {e}"),
//! }
//!
//! let _ = shutdown_tx.send(true);
//! ```
//!
//! See `examples/fetcher.rs` for a runnable version of the above.
//!
//! ---
//!
//! ## Priority scheduling
//!
//! Requests are placed in one of four FIFO lanes (`High`, `Normal`, `Low`, `Idle`)
//! and dequeued using weighted round-robin with weights **8 : 4 : 2 : 1**.
//! A request is always dispatched from the highest non-empty lane when its slot
//! comes up, so head-of-line blocking in a lower lane cannot starve it.
//!
//! ```text
//! ┌──────────┐  weight 8
//! │  High    │──────────────────────────────────────┐
//! ├──────────┤  weight 4                            │
//! │  Normal  │───────────────────────┐              │   Global semaphore
//! ├──────────┤  weight 2             │              │──▶  (global_slots)
//! │  Low     │───────────┐           │              │
//! ├──────────┤  weight 1 │           │              │   Per-origin semaphore
//! │  Idle    │───┐       │           │              │──▶  (h1/h2_per_origin)
//! └──────────┘   │       │           │              │
//!                └───────┴───────────┘──────────────┘
//! ```
//!
//! ---
//!
//! ## Request coalescing
//!
//! If two callers submit requests for the **same URL + method + Vary headers**
//! before the first response arrives, only one actual HTTP request is made.
//! The second caller is registered as a *subscriber* on the in-flight entry and
//! receives the same [`FetchResult`](types::FetchResult) when the response
//! completes.
//!
//! Coalescing is keyed by [`types::FetchKeyData::generate`], which hashes the
//! method, normalised URL, and a subset of request headers (`Range`, `Accept`,
//! `Accept-Language`, `Accept-Encoding`, and hashed `Authorization` / `Cookie`).
//! POST and other non-idempotent methods always bypass coalescing.
//!
//! ---
//!
//! ## `FetcherContext` — engine bridge
//!
//! [`fetcher_context::FetcherContext`] decouples the scheduler from engine internals
//! (tabs, event routing).  Implement it to hook into the fetch lifecycle:
//!
//! | Method | When called | Typical use |
//! |---|---|---|
//! | `observer_for` | Once per unique fetch (leader only) | Return an emitter that forwards `NetEvent`s to the correct tab |
//! | `on_ref_active` | When the leader starts a new fetch | Increment a reference counter |
//! | `on_ref_done` | When all subscribers have received their result | Decrement and clean up |
//!
//! The engine implements this via `EngineNetContext`; standalone callers can use
//! [`null_emitter::NullEmitter`] as a no-op.

pub mod decision_hub;
pub mod events;
pub mod simple;
pub mod fetch;
pub mod fetcher;
pub mod fetcher_context;
pub mod fs_utils;
pub mod null_emitter;
pub mod observer;
pub mod pump;
pub mod request_ref;
pub mod shared_body;
pub mod types;
pub mod utils;
