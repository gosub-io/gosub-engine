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
    fn t() -> TabId {
        TabId::new()
    }
    fn o(s: &str) -> url::Origin {
        let url = url::Url::parse(s).expect("valid URL");
        url.origin()
    }

    #[test]
    fn construct_local_event_without_source_tab() {
        let ev = StorageEvent {
            zone: z(),
            partition: PartitionKey::None,
            origin: o("https://example.com"),
            key: Some("greeting".into()),
            old_value: None,
            new_value: Some("hello".into()),
            source_tab: None,
            scope: StorageScope::Local,
        };

        assert!(matches!(ev.partition, PartitionKey::None));
        assert_eq!(ev.origin.ascii_serialization(), "https://example.com");
        assert_eq!(ev.key.as_deref(), Some("greeting"));
        assert_eq!(ev.old_value, None);
        assert_eq!(ev.new_value.as_deref(), Some("hello"));
        assert!(ev.source_tab.is_none());
        matches!(ev.scope, StorageScope::Local);
    }

    #[test]
    fn construct_session_event_with_source_tab_and_value_change() {
        let zone = z();
        let tab = t();

        let mut ev = StorageEvent {
            zone: zone.clone(),
            partition: PartitionKey::TopLevel(o("https://site.test")),
            origin: o("https://site.test"),
            key: Some("count".into()),
            old_value: Some("1".into()),
            new_value: Some("2".into()),
            source_tab: Some(tab),
            scope: StorageScope::Session,
        };

        // Basic checks
        match &ev.partition {
            PartitionKey::TopLevel(orig) => {
                assert_eq!(orig.ascii_serialization(), "https://site.test")
            }
            _ => panic!("expected TopLevel partition"),
        }
        assert_eq!(ev.origin.ascii_serialization(), "https://site.test");
        assert_eq!(ev.key.as_deref(), Some("count"));
        assert_eq!(ev.old_value.as_deref(), Some("1"));
        assert_eq!(ev.new_value.as_deref(), Some("2"));
        assert!(ev.source_tab.is_some());
        matches!(ev.scope, StorageScope::Session);

        // Mutate to ensure the struct is writable and fields behave as expected.
        ev.old_value = ev.new_value.clone();
        ev.new_value = Some("3".into());
        assert_eq!(ev.old_value.as_deref(), Some("2"));
        assert_eq!(ev.new_value.as_deref(), Some("3"));

        // Zone should still match the original (Clone on ZoneId works)
        assert_eq!(format!("{:?}", ev.zone), format!("{:?}", zone));
    }

    #[test]
    fn clone_event_is_independent() {
        let ev1 = StorageEvent {
            zone: z(),
            partition: PartitionKey::None,
            origin: o("http://a.test"),
            key: None,
            old_value: None,
            new_value: None,
            source_tab: Some(t()),
            scope: StorageScope::Session,
        };

        let mut ev2 = ev1.clone();
        ev2.key = Some("k".into());
        ev2.new_value = Some("v".into());

        // Original unaffected
        assert!(ev1.key.is_none());
        assert!(ev1.new_value.is_none());

        // Clone has the changes
        assert_eq!(ev2.key.as_deref(), Some("k"));
        assert_eq!(ev2.new_value.as_deref(), Some("v"));
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
