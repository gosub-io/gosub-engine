use super::area::{LocalStore, SessionStore, StorageArea};
use super::event::{StorageEvent, StorageScope};
use super::types::PartitionKey;
use crate::engine::DEFAULT_CHANNEL_CAPACITY;
use crate::tab::TabId;
use crate::zone::ZoneId;
use anyhow::Result;
use std::fmt::Debug;
use std::sync::Arc;
use tokio::sync::broadcast;

/// A handle for receiving storage change notifications.
pub type Subscription = broadcast::Receiver<StorageEvent>;

#[derive(Debug)]
struct StorageBus {
    tx: broadcast::Sender<StorageEvent>,
}

impl Default for StorageBus {
    fn default() -> Self {
        let (tx, _rx) = broadcast::channel(DEFAULT_CHANNEL_CAPACITY);
        Self { tx }
    }
}

impl StorageBus {
    fn subscribe(&self) -> Subscription {
        self.tx.subscribe()
    }
    fn publish(&self, ev: StorageEvent) {
        // broadcast::Sender::send() fails only when there are 0 receivers.
        // That’s fine: if nobody listens, we can ignore the error.
        let _ = self.tx.send(ev);
    }
}

#[derive(Clone)]
pub struct StorageService {
    local: Arc<dyn LocalStore>,
    session: Arc<dyn SessionStore>,
    bus: Arc<StorageBus>,
}

impl Debug for StorageService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorageService").finish_non_exhaustive()
    }
}

impl StorageService {
    pub fn new(local: Arc<dyn LocalStore>, session: Arc<dyn SessionStore>) -> Self {
        Self {
            local,
            session,
            bus: Arc::new(StorageBus::default()),
        }
    }

    pub fn subscribe(&self) -> Subscription {
        self.bus.subscribe()
    }

    pub fn local_for(&self, zone: ZoneId, part: &PartitionKey, origin: &url::Origin) -> Result<Arc<dyn StorageArea>> {
        let inner = self.local.area(zone, part, origin)?;
        Ok(self.wrap_notifying(
            inner,
            zone,
            None,
            part.clone(),
            origin.clone(),
            StorageScope::Local,
        ))
    }

    pub fn session_for(
        &self,
        zone: ZoneId,
        tab: TabId,
        part: &PartitionKey,
        origin: &url::Origin,
    ) -> Result<Arc<dyn StorageArea>> {
        let inner = self.session.area(zone, tab, part, origin);
        Ok(self.wrap_notifying(
            inner,
            zone,
            Some(tab),
            part.clone(),
            origin.clone(),
            StorageScope::Session,
        ))
    }

    pub fn drop_tab(&self, zone: ZoneId, tab: TabId) {
        self.session.drop_tab(zone, tab);
    }

    fn wrap_notifying(
        &self,
        inner: Arc<dyn StorageArea>,
        zone: ZoneId,
        source_tab: Option<TabId>,
        partition: PartitionKey,
        origin: url::Origin,
        scope: StorageScope,
    ) -> Arc<dyn StorageArea> {
        Arc::new(NotifyingArea {
            inner,
            zone,
            partition,
            origin,
            source_tab,
            bus: self.bus.clone(),
            scope,
        })
    }
}

struct NotifyingArea {
    inner: Arc<dyn StorageArea>,
    zone: ZoneId,
    partition: PartitionKey,
    origin: url::Origin,
    source_tab: Option<TabId>,
    bus: Arc<StorageBus>,
    scope: StorageScope,
}

impl StorageArea for NotifyingArea {
    fn get_item(&self, key: &str) -> Option<String> {
        self.inner.get_item(key)
    }
    fn set_item(&self, key: &str, value: &str) -> Result<()> {
        let old = self.inner.get_item(key);
        self.inner.set_item(key, value)?;
        self.bus.publish(StorageEvent {
            zone: self.zone,
            partition: self.partition.clone(),
            origin: self.origin.clone(),
            key: Some(key.to_string()),
            old_value: old,
            new_value: Some(value.to_string()),
            source_tab: self.source_tab,
            scope: self.scope,
        });
        Ok(())
    }
    fn remove_item(&self, key: &str) -> Result<()> {
        let old = self.inner.get_item(key);
        self.inner.remove_item(key)?;
        self.bus.publish(StorageEvent {
            zone: self.zone,
            partition: self.partition.clone(),
            origin: self.origin.clone(),
            key: Some(key.to_string()),
            old_value: old,
            new_value: None,
            source_tab: self.source_tab,
            scope: self.scope,
        });
        Ok(())
    }
    fn clear(&self) -> Result<()> {
        self.inner.clear()?;
        self.bus.publish(StorageEvent {
            zone: self.zone,
            partition: self.partition.clone(),
            origin: self.origin.clone(),
            key: None,
            old_value: None,
            new_value: None,
            source_tab: self.source_tab,
            scope: self.scope,
        });
        Ok(())
    }
    fn len(&self) -> usize {
        self.inner.len()
    }
    fn keys(&self) -> Vec<String> {
        self.inner.keys()
    }
}
