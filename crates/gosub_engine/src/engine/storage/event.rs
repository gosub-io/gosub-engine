use super::types::PartitionKey;
use crate::tab::TabId;
use crate::zone::ZoneId;

#[derive(Copy, Clone, Debug)]
pub enum StorageScope {
    Local,
    Session,
}

/// A change to a storage area, published on the zone's event bus.
#[derive(Clone, Debug)]
pub struct StorageEvent {
    pub zone: ZoneId,
    pub partition: PartitionKey,
    pub origin: url::Origin,
    /// `None` means the whole area was cleared.
    pub key: Option<String>,
    pub old_value: Option<String>,
    pub new_value: Option<String>,
    pub source_tab: Option<TabId>,
    pub scope: StorageScope,
}
