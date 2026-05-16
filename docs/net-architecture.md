Looking at the code structure, here's a schematic diagram of how the net module works:

```ascii
┌─────────────────────────────────────────────────────────────────────────┐
│                              ENGINE LAYER                               │
│  ┌─────────────┐    ┌──────────────┐    ┌─────────────────────────────┐ │
│  │    Zone     │    │     Tab      │    │      EngineContext          │ │
│  │  (Browser   │    │  (Document   │    │   - event_tx (broadcast)    │ │
│  │  Context)   │    │   Context)   │    │   - request_reference_map   │ │
│  └─────┬───────┘    └──────┬───────┘    └─────────────┬───────────────┘ │
│        │                   │                          │                 │
└────────┼───────────────────┼──────────────────────────┼─────────────────┘
         │                   │                          │
         │                   │                          │
┌────────▼───────────────────▼──────────────────────────▼──────────────────┐
│                           IO RUNTIME LAYER                               │
│                                                                          │
│  ┌─────────────────────────────────────────────────────────────────────┐ │
│  │                        IoHandle                                     │ │
│  │  - tx_submit: IoChannel (mpsc sender)                               │ │
│  │  - shutdown_tx: watch::Sender<bool>                                 │ │
│  │  - join_handle: JoinHandle                                          │ │
│  └─────────────────────────────────────────────────────────────────────┘ │
│                                   │                                      │
│                            ┌──────▼──────┐                               │
│                            │ IoCommand      │                            │
│                            │ - Fetch        │                            │
│                            │ - Decision     │                            │
│                            │ - ShutdownZone │                            │
│                            └──────┬──────┘                               │
│                                   │                                      │
│  ┌─────────────────────────────────▼──────────────────────────────────┐  │
│  │                          IoRouter                                  │  │
│  │  - zones: DashMap<ZoneId, ZoneEntry>                               │  │
│  │  - cfg: FetcherConfig                                              │  │
│  │  - engine_ctx: Arc<EngineContext>                                  │  │
│  │                                                                    │  │
│  │  Routes requests to zone-specific fetchers                         │  │
│  └─────┬───────────────────────────────────┬──────────────────────────┘  │
│        │                                   │                             │
└────────┼───────────────────────────────────┼─────────────────────────────┘
         │                                   │
         │ Zone A                            │ Zone B
         │                                   │
┌────────▼─────────────┐            ┌────────▼─────────────┐
│    ZoneEntry         │            │    ZoneEntry         │
│  - fetcher: Fetcher  │            │  - fetcher: Fetcher  │
│  - shutdown_tx       │            │  - shutdown_tx       │
│  - join              │            │  - join              │
└────────┬─────────────┘            └────────┬─────────────┘
         │                                   │
┌────────▼─────────────┐            ┌────────▼─────────────┐
│      FETCHER         │            │      FETCHER         │
│                      │            │                      │
│ Priority Queues:     │            │ Priority Queues:     │
│  - q_high            │            │  - q_high            │
│  - q_norm            │            │  - q_norm            │
│  - q_low             │            │  - q_low             │
│  - q_idle            │            │  - q_idle            │
│                      │            │                      │
│ Slot Management:     │            │ Slot Management:     │
│  - global_slots      │            │  - global_slots      │
│  - per_origin        │            │  - per_origin        │
│                      │            │                      │
│ Request Coalescing:  │            │ Request Coalescing:  │
│  - inflight: DashMap │            │  - inflight: DashMap │
│                      │            │                      │
│ HTTP Client:         │            │ HTTP Client:         │
│  - reqwest::Client   │            │  - reqwest::Client   │
└────────┬─────────────┘            └──────────────┬───────┘
         │                                         │
┌────────▼─────────────────────────────────────────▼─────────────┐
│                    NETWORK LAYER                               │
│                                                                │
│  ┌──────────────┐  ┌──────────────┐  ┌─────────────────────┐   │
│  │    fetch     │  │ shared_body  │  │   decision_hub      │   │
│  │  (HTTP I/O)  │  │ (Streaming)  │  │ (Content sniffing)  │   │
│  └──────────────┘  └──────────────┘  └─────────────────────┘   │
│                                                                │
│  ┌──────────────┐  ┌──────────────┐  ┌─────────────────────┐   │
│  │    router    │  │    events    │  │      types          │   │
│  │ (Protocol)   │  │ (Observer)   │  │   (Data structs)    │   │
│  └──────────────┘  └──────────────┘  └─────────────────────┘   │
└────────────────────────────────────────────────────────────────┘
```

**Key Flow:**

1. **Request Submission**: Zones/Tabs submit `FetchRequest` via `IoChannel`
2. **Routing**: `IoRouter` routes requests to zone-specific `Fetcher` instances
3. **Prioritization**: Each `Fetcher` queues requests by priority (High→Normal→Low→Idle)
4. **Coalescing**: Identical requests are coalesced using `inflight` map
5. **Slot Management**: Global and per-origin concurrency limits are enforced
6. **Execution**: HTTP requests are made via `reqwest::Client`
7. **Response Handling**: Results are returned as either buffered or streaming

**Isolation**: Each zone gets its own `Fetcher` instance, providing complete isolation between browser contexts.
Each `Fetcher` maintains its own `global_slots` semaphore and per-origin semaphores — concurrency limits are
enforced independently per zone, not shared across zones.

```