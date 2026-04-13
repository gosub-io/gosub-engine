use super::types::PartitionKey;
use gosub_net::types::TabId;
use gosub_net::types::ZoneId;

#[derive(Copy, Clone, Debug)]
pub enum StorageScope {
    Local,
    Session,
}

#[derive(Clone, Debug)]
pub struct StorageEvent {
    pub zone: ZoneId,
    pub partition: PartitionKey,
    pub origin: url::Origin,
    pub key: Option<String>,
    pub old_value: Option<String>,
    pub new_value: Option<String>,
    pub source_tab: Option<TabId>,
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

        ev.old_value = ev.new_value.clone();
        ev.new_value = Some("3".into());
        assert_eq!(ev.old_value.as_deref(), Some("2"));
        assert_eq!(ev.new_value.as_deref(), Some("3"));

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

        assert!(ev1.key.is_none());
        assert!(ev1.new_value.is_none());
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
        let expected_substrings = ["StorageEvent", "Local", &format!("{origin_url:?}"), "key: Some(\"x\")"];

        for &needle in &expected_substrings {
            assert!(
                s.contains(needle),
                "Expected debug output to contain `{needle}`, but got:\n{s}"
            );
        }
    }
}
