use crate::net::types::{FetchHandle, FetchRequest, FetchResult};
use crate::net::FetchInflightMap;
use crate::tab::TabId;
use crate::NavigationId;
use std::collections::HashMap;

/// Unique key for a navigation in a tab
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NavKey {
    pub tab_id: TabId,
    pub nav_id: NavigationId,
}

impl NavKey {
    pub(crate) fn new(tab_id: TabId, nav_id: NavigationId) -> Self {
        Self {
            tab_id,
            nav_id,
        }
    }
}

/// Represents an in-flight navigation operation. It allows for easy cancellation in case
/// the navigation is no longer needed (e.g., user navigated away).
pub(crate) struct NavInflightEntry {
    /// Unique key for this navigation
    pub key: NavKey,
    /// Fetch handle for this navigation
    pub fetch: FetchHandle,
    // /// User agent decision for this navigation, if any
    // pub ua_decision: Option<UaDecision>,
}

/// Current in-flight navigations for a tab
#[derive(Default)]
pub(crate) struct NavInflightMap {
    map: HashMap<NavKey, NavInflightEntry>,
}

impl NavInflightMap {
    /// Starts a new navigation or joins an existing one if the same request is already in-flight.
    pub fn start_navigation(
        &mut self,
        nav_key: NavKey,
        req: FetchRequest,
        wants_stream: bool,
        fetch_inflight: &FetchInflightMap,
    ) -> (tokio::sync::oneshot::Receiver<FetchResult>, FetchHandle) {

        if let Some(existing) = self.map.get(&nav_key) {
            // If there's already an in-flight navigation for this key, join it
            let (handle, rx, _new) = fetch_inflight.join_or_start(&req, wants_stream);
            return (rx, handle);
        }

        // Create a new fetch
        let (handle, rx, _new) = fetch_inflight.join_or_start(&req, wants_stream);

        // Insert or update the entry
        self.map.insert(
            nav_key,
            NavInflightEntry {
                key: nav_key,
                fetch: handle.clone(),
            },
        );
        (rx, handle)
    }

    /// Restarts a navigation by cancelling any existing one and starting a new fetch.
    pub fn restart_navigation(
        &mut self,
        nav_key: NavKey,
        req: FetchRequest,
        wants_stream: bool,
        fetch_inflight: &FetchInflightMap,
    ) -> (tokio::sync::oneshot::Receiver<FetchResult>, FetchHandle) {
        // Cancel existing navigation if it exists
        if let Some(old) = self.map.remove(&nav_key) {
            old.fetch.cancel.cancel();
        }
        // Start a new navigation
        let (handle, rx, _new) = fetch_inflight.join_or_start(&req, wants_stream);

        // Insert the new entry
        self.map.insert(
            nav_key,
            NavInflightEntry {
                key: nav_key,
                fetch: handle.clone(),
            },
        );
        (rx, handle)
    }

    /// Cancels an in-flight navigation if it exists. Will cancel all subscribers
    pub fn cancel_navigation(&mut self, nav_key: &NavKey) {
        if let Some(e) = self.map.get(nav_key) {
            // child cancel: drops this subscriber; fetch may continue
            e.fetch.cancel.cancel();
        }
        self.map.remove(nav_key);
    }

    pub fn complete_navigation(&mut self, nav_key: &NavKey) {
        self.map.remove(nav_key);
    }
}
