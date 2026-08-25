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
    pub fn new(fork_server: Arc<Mutex<ForkServer>>) -> Self {
        Self {
            fork_server,
            state: Mutex::new(State::default()),
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
            log::warn!("renderer for {} in zone {:?} died; replacing it", key.site, key.zone);
            self.discard(&mut state, &key);
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
