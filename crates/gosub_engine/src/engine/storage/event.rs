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
