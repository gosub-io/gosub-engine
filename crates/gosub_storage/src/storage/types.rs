use gosub_net::types::ZoneId;
use url::{Origin, Url};
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
#[derive(Default)]
pub enum PartitionKey {
    #[default]
    None,
    TopLevel(Origin),
    Custom(String),
}


impl std::str::FromStr for PartitionKey {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            Ok(PartitionKey::None)
        } else {
            let Ok(url) = Url::parse(s) else {
                return Ok(PartitionKey::Custom(s.to_string()));
            };
            Ok(PartitionKey::TopLevel(url.origin()))
        }
    }
}

impl PartitionKey {
    pub fn random() -> Self {
        let random = Uuid::new_v4();
        PartitionKey::Custom(random.to_string())
    }

    pub fn from_zone(zone_id: ZoneId) -> Self {
        format!("https://zone-{}.local", zone_id).parse().unwrap()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum PartitionPolicy {
    None,
    #[default]
    TopLevelOrigin,
}

pub fn compute_partition_key(u: &Url, p: PartitionPolicy) -> PartitionKey {
    match p {
        PartitionPolicy::None => PartitionKey::None,
        PartitionPolicy::TopLevelOrigin => PartitionKey::TopLevel(u.origin()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use url::Url;

    fn o(s: &str) -> Origin {
        let url = Url::parse(s).expect("valid URL");
        url.origin()
    }

    #[test]
    fn partitionkey_default_is_none() {
        let pk: PartitionKey = Default::default();
        assert_eq!(pk, PartitionKey::None);
    }

    #[test]
    fn compute_none_policy_returns_none() {
        let u = Url::parse("https://example.com/path?q=1#frag").unwrap();
        assert_eq!(compute_partition_key(&u, PartitionPolicy::None), PartitionKey::None);
    }

    #[test]
    fn compute_toplevel_uses_origin_ascii_serialization_with_non_default_port() {
        let u = Url::parse("https://sub.example.com:8443/path?q=1#f").unwrap();
        let pk = compute_partition_key(&u, PartitionPolicy::TopLevelOrigin);
        match pk {
            PartitionKey::TopLevel(o) => {
                assert_eq!(o.ascii_serialization(), "https://sub.example.com:8443");
            }
            _ => panic!("expected TopLevel origin"),
        }
    }

    #[test]
    fn compute_toplevel_elides_default_port() {
        let u = Url::parse("https://example.com/anything").unwrap();
        let pk = compute_partition_key(&u, PartitionPolicy::TopLevelOrigin);
        assert_eq!(pk, PartitionKey::TopLevel(o("https://example.com")));
    }

    #[test]
    fn compute_toplevel_ipv6_with_port() {
        let u = Url::parse("http://[2001:db8::1]:8080/").unwrap();
        let pk = compute_partition_key(&u, PartitionPolicy::TopLevelOrigin);
        assert_eq!(pk, "http://[2001:db8::1]:8080".parse::<PartitionKey>().unwrap());
    }

    #[test]
    fn partitionkey_equality_and_hash_semantics() {
        use std::collections::HashSet;

        let a = "https://a.example".parse::<PartitionKey>().unwrap();
        let b = "https://a.example".parse::<PartitionKey>().unwrap();
        let c = "https://b.example".parse::<PartitionKey>().unwrap();
        let none = PartitionKey::None;

        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, none);

        let mut set = HashSet::new();
        set.insert(a.clone());
        set.insert(b.clone());
        set.insert(c.clone());
        set.insert(none.clone());

        assert!(set.contains(&a));
        assert!(set.contains(&b));
        assert!(set.contains(&c));
        assert!(set.contains(&none));
        assert_eq!(set.len(), 3);
    }
}
