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
}

pub struct RendererPool {
    fork_server: Arc<Mutex<ForkServer>>,
    state: Mutex<State>,
    /// When [`Self::sweep_dead`] last looked; it is called per frame per tab.
    last_sweep: Mutex<Option<std::time::Instant>>,
    /// Where a crash is announced (`EngineEvent::RendererCrashed`), if the
    /// pool belongs to an engine.
    events: Option<crate::engine::types::EventChannel>,
}

#[derive(Default)]
struct State {
    renderers: HashMap<RendererKey, Arc<Mutex<ResidentRenderer>>>,
    tabs: HashMap<RendererKey, HashSet<TabId>>,
    /// Where each tab currently lives.
    placement: HashMap<TabId, RendererKey>,
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
        if state.renderers.get(&key).is_some_and(|r| r.lock().is_dead()) {
            self.bury(&mut state, &key);
        }

        let renderer = match state.renderers.get(&key) {
            Some(renderer) => Arc::clone(renderer),
            None => {
                // The `ps` name wants the host, not the scheme.
                let label = site.rsplit("://").next().unwrap_or(site);
                let renderer = Arc::new(Mutex::new(self.fork_server.lock().spawn_renderer(label)?));
                state.renderers.insert(key.clone(), Arc::clone(&renderer));
                renderer
            }
        };
        if state.placement.get(&tab) != Some(&key) {
            renderer.lock().open_tab(&tab.to_string())?;
            state.tabs.entry(key.clone()).or_default().insert(tab);
            state.placement.insert(tab, key);
        }
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
        dead.len()
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

    /// Every running renderer and how many tabs it hosts.
    pub fn snapshot(&self) -> Vec<RendererStatus> {
        let state = self.state.lock();
        let mut out: Vec<RendererStatus> = state
            .renderers
            .iter()
            .map(|(key, renderer)| RendererStatus {
                key: key.clone(),
                pid: renderer.lock().pid(),
                tabs: state.tabs.get(key).map_or(0, HashSet::len),
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

    fn detach(&self, state: &mut State, key: &RendererKey, tab: TabId) {
        state.placement.remove(&tab);
        let last = {
            let tabs = state.tabs.entry(key.clone()).or_default();
            tabs.remove(&tab);
            tabs.is_empty()
        };
        if let Some(renderer) = state.renderers.get(key) {
            let _ = renderer.lock().close_tab(&tab.to_string());
        }
        if last {
            self.discard(state, key);
        }
    }

    fn discard(&self, state: &mut State, key: &RendererKey) {
        state.tabs.remove(key);
        if let Some(renderer) = state.renderers.remove(key) {
            renderer.lock().shutdown();
        }
        self.fork_server.lock().reap_exited();
    }
}
