//! Resident renderers, one per (zone, site), shared by that site's tabs.
//!
//! The site is Chromium's (scheme + eTLD+1, see [`super::site`]): tabs on one
//! site in one zone share a process, a tab that navigates cross-site moves to
//! another, and a process whose last tab leaves is shut down. A process that
//! died is noticed on the next request for it and replaced.

use crate::fork_server::client::{ForkServer, ResidentRenderer};
use crate::tab::TabId;
use crate::zone::ZoneId;
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// What one renderer process serves.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RendererKey {
    pub zone: ZoneId,
    pub site: String,
}

/// One running renderer, for anyone listing the pool.
#[derive(Clone, Debug)]
pub struct RendererStatus {
    pub key: RendererKey,
    pub pid: i32,
    pub tabs: usize,
    /// Resident set and data segment in KiB, from `/proc`, when readable.
    pub rss_kb: Option<u64>,
    pub data_kb: Option<u64>,
}

/// `VmRSS` and `VmData` of a process in KiB, from `/proc/<pid>/status`.
pub fn memory_of(pid: i32) -> Option<(u64, u64)> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let field = |name: &str| -> Option<u64> {
        status
            .lines()
            .find(|l| l.starts_with(name))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse().ok())
    };
    Some((field("VmRSS:")?, field("VmData:")?))
}

pub struct RendererPool {
    fork_server: Arc<Mutex<ForkServer>>,
    state: Mutex<State>,
    /// When [`Self::sweep_dead`] last looked; it is called per frame per tab.
    last_sweep: Mutex<Option<std::time::Instant>>,
    /// When memory was last reported to the firehose.
    last_memory_report: Mutex<Option<std::time::Instant>>,
    /// Where a crash is announced (`EngineEvent::RendererCrashed`), if the
    /// pool belongs to an engine.
    events: Option<crate::engine::types::EventChannel>,
}

#[derive(Default)]
struct State {
    renderers: HashMap<RendererKey, Arc<Mutex<ResidentRenderer>>>,
    /// Each renderer's pid, kept here so listing the pool never waits on a
    /// renderer that is mid-exchange.
    pids: HashMap<RendererKey, i32>,
    /// Each renderer's dead flag, readable without its lock: a renderer that
    /// died mid-exchange must never be handed to the next tab.
    dead_flags: HashMap<RendererKey, Arc<std::sync::atomic::AtomicBool>>,
    tabs: HashMap<RendererKey, HashSet<TabId>>,
    /// Where each tab currently lives.
    placement: HashMap<TabId, RendererKey>,
    /// `OpenTab`s / `CloseTab`s not yet delivered: the renderer was busy when
    /// the tab arrived or left, and the pool never waits on a renderer.
    pending_opens: HashMap<RendererKey, Vec<TabId>>,
    pending_closes: HashMap<RendererKey, Vec<TabId>>,
    /// A renderer was let go of while busy; its exit needs collecting later.
    needs_reap: bool,
}

impl std::fmt::Debug for RendererPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RendererPool")
            .field("renderers", &self.state.lock().renderers.len())
            .finish_non_exhaustive()
    }
}

impl RendererPool {
    pub fn new(fork_server: Arc<Mutex<ForkServer>>, events: Option<crate::engine::types::EventChannel>) -> Self {
        Self {
            fork_server,
            state: Mutex::new(State::default()),
            last_sweep: Mutex::new(None),
            last_memory_report: Mutex::new(None),
            events,
        }
    }

    pub fn fork_server(&self) -> &Arc<Mutex<ForkServer>> {
        &self.fork_server
    }

    /// The renderer `tab` renders `site` in: spawned if the site has none,
    /// replaced if it died, and `tab` moved into it if it lived elsewhere.
    /// The returned lock is held by the caller for the render - requests to
    /// one process are strictly serial, so same-site tabs take turns.
    pub fn renderer_for(&self, zone: ZoneId, site: &str, tab: TabId) -> anyhow::Result<Arc<Mutex<ResidentRenderer>>> {
        let key = RendererKey {
            zone,
            site: site.to_string(),
        };
        let mut state = self.state.lock();

        if let Some(old) = state.placement.get(&tab).cloned() {
            if old != key {
                self.detach(&mut state, &old, tab);
            }
        }
        // Lock-free on purpose: the pool lock is held, and a renderer that
        // died mid-exchange is exactly one whose lock is still taken by the
        // failing exchange - it must be replaced now, not handed out again.
        if state
            .dead_flags
            .get(&key)
            .is_some_and(|dead| dead.load(std::sync::atomic::Ordering::Acquire))
        {
            self.bury(&mut state, &key);
        }

        let renderer = match state.renderers.get(&key) {
            Some(renderer) => Arc::clone(renderer),
            None => {
                // Spawning is a round trip to the fork server: other tabs'
                // lookups must not wait on it, so the pool lock is let go and
                // retaken. Two tabs racing for one new site both spawn; the
                // loser's renderer is dropped unused.
                drop(state);
                // The `ps` name wants the host, not the scheme.
                let label = site.rsplit("://").next().unwrap_or(site);
                let spawned = self.fork_server.lock().spawn_renderer(label)?;
                state = self.state.lock();
                match state.renderers.get(&key) {
                    Some(renderer) => Arc::clone(renderer),
                    None => {
                        state.pids.insert(key.clone(), spawned.pid());
                        state.dead_flags.insert(key.clone(), spawned.dead_flag());
                        let renderer = Arc::new(Mutex::new(spawned));
                        state.renderers.insert(key.clone(), Arc::clone(&renderer));
                        renderer
                    }
                }
            }
        };
        if state.placement.get(&tab) != Some(&key) {
            state.tabs.entry(key.clone()).or_default().insert(tab);
            state.placement.insert(tab, key.clone());
            // Announced when the caller's own lock on the renderer is taken:
            // the pool must not wait for a renderer mid-exchange for another tab.
            state.pending_opens.entry(key).or_default().push(tab);
        }
        drop(state);
        self.flush_pending(&renderer);
        Ok(renderer)
    }

    /// `tab` is gone: tell its renderer, and shut the renderer down if that
    /// was its last tab.
    pub fn release(&self, tab: TabId) {
        let mut state = self.state.lock();
        if let Some(key) = state.placement.get(&tab).cloned() {
            self.detach(&mut state, &key, tab);
        }
    }

    /// Notice renderers that died since the last look - a closed link, seen
    /// without sending anything - and replace them on their tabs' next
    /// request, announcing each now rather than then. Cheap enough to call
    /// per frame: it actually looks at most every quarter second, and skips
    /// a renderer that is busy with an exchange (one that dies mid-exchange
    /// is found by the exchange). Returns how many were found dead.
    pub fn sweep_dead(&self) -> usize {
        const INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);
        {
            let mut last = self.last_sweep.lock();
            if last.is_some_and(|t| t.elapsed() < INTERVAL) {
                return 0;
            }
            *last = Some(std::time::Instant::now());
        }
        let mut state = self.state.lock();
        let dead: Vec<RendererKey> = state
            .renderers
            .iter()
            .filter(|(_, renderer)| renderer.try_lock().is_some_and(|mut r| !r.check_alive()))
            .map(|(key, _)| key.clone())
            .collect();
        for key in &dead {
            self.bury(&mut state, key);
        }
        if std::mem::take(&mut state.needs_reap) {
            self.fork_server.lock().reap_exited();
        }
        let idle: Vec<Arc<Mutex<ResidentRenderer>>> = state.renderers.values().map(Arc::clone).collect();
        drop(state);
        for renderer in idle {
            self.flush_pending(&renderer);
        }
        self.report_memory();
        dead.len()
    }

    /// Every renderer's memory onto the firehose, every couple of seconds -
    /// what a long session does to a long-lived process.
    fn report_memory(&self) {
        const INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);
        if !crate::telemetry::enabled() {
            return;
        }
        {
            let mut last = self.last_memory_report.lock();
            if last.is_some_and(|t| t.elapsed() < INTERVAL) {
                return;
            }
            *last = Some(std::time::Instant::now());
        }
        for status in self.snapshot() {
            crate::telemetry::emit(
                "renderer.memory",
                serde_json::json!({
                    "pid": status.pid,
                    "site": status.key.site,
                    "tabs": status.tabs,
                    "rss_kb": status.rss_kb,
                    "data_kb": status.data_kb,
                }),
            );
        }
    }

    /// A renderer found dead: drop it, reap it, and say so.
    fn bury(&self, state: &mut State, key: &RendererKey) {
        log::warn!(
            "renderer for {} in zone {:?} died; its tabs get a fresh one",
            key.site,
            key.zone
        );
        let tabs: Vec<TabId> = state
            .tabs
            .get(key)
            .map(|t| t.iter().copied().collect())
            .unwrap_or_default();
        self.discard(state, key);
        if let Some(events) = &self.events {
            let _ = events.send(crate::engine::events::EngineEvent::RendererCrashed {
                zone_id: key.zone,
                site: key.site.clone(),
                tabs,
                error: "renderer process died".into(),
            });
        }
    }

    /// Kill every renderer serving `site`, in any zone, the way a crash
    /// would; returns how many were told to die. Their tabs recover on their
    /// next render. For tests.
    pub fn crash_renderers_for_test(&self, site: &str) -> usize {
        let state = self.state.lock();
        let mut count = 0;
        for (key, renderer) in &state.renderers {
            if key.site != site {
                continue;
            }
            renderer.lock().crash_for_test();
            count += 1;
        }
        count
    }

    /// The escape audit in the fork server, a renderer forked for it, and
    /// every resident renderer, labelled.
    pub fn audit(&self) -> Vec<(String, anyhow::Result<gosub_sandbox::audit::AuditReport>)> {
        let mut out = Vec::new();
        {
            let mut server = self.fork_server.lock();
            out.push(("fork-server".to_string(), server.audit()));
            out.push(("forked renderer".to_string(), server.audit_forked_renderer()));
        }
        let renderers: Vec<(RendererKey, Arc<Mutex<ResidentRenderer>>)> = self
            .state
            .lock()
            .renderers
            .iter()
            .map(|(k, r)| (k.clone(), Arc::clone(r)))
            .collect();
        for (key, renderer) in renderers {
            out.push((format!("renderer {}", key.site), renderer.lock().audit()));
        }
        out
    }

    /// Every running renderer and how many tabs it hosts.
    pub fn snapshot(&self) -> Vec<RendererStatus> {
        // The `/proc` reads happen after the lock is released.
        let listed: Vec<(RendererKey, i32, usize)> = {
            let state = self.state.lock();
            state
                .renderers
                .keys()
                .map(|key| {
                    (
                        key.clone(),
                        state.pids.get(key).copied().unwrap_or(0),
                        state.tabs.get(key).map_or(0, HashSet::len),
                    )
                })
                .collect()
        };
        let mut out: Vec<RendererStatus> = listed
            .into_iter()
            .map(|(key, pid, tabs)| {
                let memory = memory_of(pid);
                RendererStatus {
                    key,
                    pid,
                    tabs,
                    rss_kb: memory.map(|m| m.0),
                    data_kb: memory.map(|m| m.1),
                }
            })
            .collect();
        out.sort_by(|a, b| a.key.site.cmp(&b.key.site));
        out
    }

    /// Shut every renderer down (engine shutdown).
    pub fn shutdown_all(&self) {
        let mut state = self.state.lock();
        let keys: Vec<RendererKey> = state.renderers.keys().cloned().collect();
        for key in keys {
            self.discard(&mut state, &key);
        }
        state.placement.clear();
    }

    /// Deliver what a renderer could not be told while it was busy. Called
    /// with no pool lock held, by whoever is about to use the renderer. Lock
    /// order is strictly pool → renderer (try), never the reverse: the
    /// pending messages are collected first, and pushed back if the renderer
    /// turns out to be mid-exchange.
    fn flush_pending(&self, renderer: &Arc<Mutex<ResidentRenderer>>) {
        let (key, opens, closes) = {
            let mut state = self.state.lock();
            let key = state
                .renderers
                .iter()
                .find(|(_, r)| Arc::ptr_eq(r, renderer))
                .map(|(k, _)| k.clone());
            let Some(key) = key else {
                return;
            };
            let opens = state.pending_opens.remove(&key).unwrap_or_default();
            let closes = state.pending_closes.remove(&key).unwrap_or_default();
            (key, opens, closes)
        };
        if opens.is_empty() && closes.is_empty() {
            return;
        }
        match renderer.try_lock() {
            Some(mut guard) => {
                for tab in &opens {
                    let _ = guard.open_tab(&tab.to_string());
                }
                for tab in &closes {
                    let _ = guard.close_tab(&tab.to_string());
                }
            }
            None => {
                // Mid-exchange: put them back for the next opportunity.
                let mut state = self.state.lock();
                let front = state.pending_opens.entry(key.clone()).or_default();
                for (i, tab) in opens.into_iter().enumerate() {
                    front.insert(i, tab);
                }
                let front = state.pending_closes.entry(key).or_default();
                for (i, tab) in closes.into_iter().enumerate() {
                    front.insert(i, tab);
                }
            }
        }
    }

    fn detach(&self, state: &mut State, key: &RendererKey, tab: TabId) {
        state.placement.remove(&tab);
        let last = {
            let tabs = state.tabs.entry(key.clone()).or_default();
            tabs.remove(&tab);
            tabs.is_empty()
        };
        // Told now if the renderer is free, later if it is mid-exchange -
        // the pool never waits on a renderer.
        if let Some(renderer) = state.renderers.get(key) {
            match renderer.try_lock() {
                Some(mut guard) => {
                    let _ = guard.close_tab(&tab.to_string());
                }
                None => state.pending_closes.entry(key.clone()).or_default().push(tab),
            }
        }
        if last {
            self.discard(state, key);
        }
    }

    fn discard(&self, state: &mut State, key: &RendererKey) {
        state.tabs.remove(key);
        state.pids.remove(key);
        state.dead_flags.remove(key);
        state.pending_opens.remove(key);
        state.pending_closes.remove(key);
        if let Some(renderer) = state.renderers.remove(key) {
            match renderer.try_lock() {
                Some(mut guard) => guard.shutdown(),
                // Busy with an exchange: the thread running it holds the last
                // handle; when that drops, the link closes and the process
                // exits on end-of-file. Its exit is collected by the sweep.
                None => {
                    state.needs_reap = true;
                    return;
                }
            }
        }
        self.fork_server.lock().reap_exited();
    }
}
