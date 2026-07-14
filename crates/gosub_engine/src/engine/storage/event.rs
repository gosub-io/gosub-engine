use super::types::PartitionKey;
use crate::tab::TabId;
use crate::zone::ZoneId;

/// Scope of the store
#[derive(Copy, Clone, Debug)]
pub enum StorageScope {
    /// Local storage, typically not tied to a specific tab.
    Local,
    /// Session storage, tied to a specific tab and valid only for the duration of that tab's session.
    Session,
}

/// Represents a storage event that occurred in a specific zone and partition, with details about the change.
#[derive(Clone, Debug)]
pub struct StorageEvent {
    /// The zone in which the storage event occurred.
    pub zone: ZoneId,
    /// The partition key indicating the storage partition (e.g., top-level origin).
    pub partition: PartitionKey,
    /// The origin of the URL where the storage event occurred.
    pub origin: url::Origin,
    /// The key that was changed in the storage.
    pub key: Option<String>,
    /// The old value of the key before the change, if applicable.
    pub old_value: Option<String>,
    /// The new value of the key after the change, if applicable.
    pub new_value: Option<String>,
    /// The tab ID that triggered the storage event, if applicable.
    pub source_tab: Option<TabId>,
    /// The scope of the storage event, indicating whether it is local or session storage.
    pub scope: StorageScope,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn z() -> ZoneId {
        ZoneId::new()
    }
    fn o(s: &str) -> url::Origin {
        let url = url::Url::parse(s).expect("valid URL");
        url.origin()
    }

    #[test]
    fn debug_includes_scope_and_origin() {
        let origin_url = o("https://debug.test");

        let ev = StorageEvent {
            zone: z(),
            partition: PartitionKey::None,
            origin: origin_url.clone(),
            key: Some("x".into()),
            old_value: Some("1".into()),
            new_value: Some("2".into()),
            source_tab: None,
            scope: StorageScope::Local,
        };

        let s = format!("{ev:?}");

        let expected_substrings = [
            "StorageEvent",
            "Local",
            &format!("{origin_url:?}"), // Use the same Debug format as the struct
            "key: Some(\"x\")",
        ];

        for &needle in &expected_substrings {
            assert!(
                s.contains(needle),
                "Expected debug output to contain `{needle}`, but got:\n{s}"
            );
        }
    }
}
