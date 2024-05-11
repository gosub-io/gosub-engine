use crate::dns::{DnsCache, DnsEntry, DnsResolver, ResolveType};
use crate::errors::Error;
use gosub_shared::types::Result;
use log::trace;
use std::collections::{HashMap, VecDeque};

pub struct CacheResolver {
    values: HashMap<String, DnsEntry>,
    max_entries: usize,
    lru: VecDeque<String>,
}

impl DnsResolver for CacheResolver {
    fn resolve(&mut self, domain: &str, resolve_type: ResolveType) -> Result<DnsEntry> {
        if let Some(entry) = self.values.get(domain) {
            if !entry.has_ipv4 && !entry.has_ipv6 && resolve_type == ResolveType::Both {
                trace!("{}: no addresses found in entry", domain);
                return Err(Error::DnsNoIpAddressFound.into());
            }
            if !entry.has_ipv4 && resolve_type == ResolveType::Ipv4 {
                trace!("{}: no ipv4 addresses found in entry", domain);
                return Err(Error::DnsNoIpAddressFound.into());
            }
            if !entry.has_ipv6 && resolve_type == ResolveType::Ipv6 {
                trace!("{}: no ipv6 addresses found in entry", domain);
                return Err(Error::DnsNoIpAddressFound.into());
            }

            trace!("{}: found in cache with correct resolve type", domain);
            self.lru.retain(|x| x != domain);
            self.lru.push_back(domain.to_string());

            return Ok(entry.clone());
        }

        Err(Error::DnsNoIpAddressFound.into())
    }

    /// When a domain is resolved, it will be announced to all resolvers. This cache resolver
    /// will store it into the cache.
    fn announce(&mut self, domain: &str, entry: &DnsEntry) {
        trace!("{}: announcing to cache", domain);

        self.lru.retain(|x| x != domain);
        self.lru.push_back(domain.to_string());

        if let Some(current_entry) = self.values.get_mut(domain) {
            trace!("{}: updating existing entry to cache", domain);

            trace!("current entries: {:?}", current_entry.ips);
            trace!("new entries: {:?}", entry.ips);
            current_entry.has_ipv4 |= entry.has_ipv4;
            current_entry.has_ipv6 |= entry.has_ipv6;

            for ip in &entry.ips {
                if current_entry.ips.iter().any(|x| x == ip) {
                    continue;
                }
                current_entry.ips.push(*ip);
            }
            trace!("merged entries: {:?}", current_entry.ips);
        } else {
            trace!("adding new entry to cache");

            // Clear out if we have too many entries
            if self.values.len() >= self.max_entries {
                if let Some(key) = self.lru.pop_front() {
                    self.values.remove(&key);
                }
            }

            self.values.insert(domain.to_string(), entry.clone());
        }
    }

    fn name(&self) -> &'static str {
        "cache resolver"
    }
}

impl DnsCache for CacheResolver {
    fn flush_all(&mut self) {
        self.values.clear();
        self.lru.clear();
    }

    fn flush_entry(&mut self, domain: &str) {
        self.values.remove(domain);
        self.lru.retain(|x| x != domain);
    }
}

impl CacheResolver {
    pub(crate) fn new(max_entries: usize) -> Self {
        Self {
            values: HashMap::with_capacity(max_entries),
            max_entries,
            lru: VecDeque::with_capacity(max_entries),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::dns::DnsEntry;

    #[test]
    fn test_cache() {
        let mut cache = CacheResolver::new(3);

        cache.announce(
            "example.com",
            &DnsEntry::new("example.com", vec!["127.0.0.1"]),
        );
        cache.announce(
            "example.org",
            &DnsEntry::new("example.org", vec!["127.0.0.1"]),
        );

        assert_eq!(cache.values.len(), 2);
        assert_eq!(cache.lru.len(), 2);
        assert_eq!(cache.lru.capacity(), 3);
        assert_eq!(cache.lru[0], "example.com");
        assert_eq!(cache.lru[1], "example.org");

        cache.announce(
            "example.net",
            &DnsEntry::new("example.net", vec!["127.0.0.1"]),
        );
        assert_eq!(cache.values.len(), 3);
        assert_eq!(cache.lru.len(), 3);
        assert_eq!(cache.lru[0], "example.com");
        assert_eq!(cache.lru[1], "example.org");
        assert_eq!(cache.lru[2], "example.net");

        println!("lru: {:?}", cache.lru);
        let _ = cache.resolve("example.org", ResolveType::Both);
        println!("lru: {:?}", cache.lru);
        assert_eq!(cache.lru[0], "example.com");
        assert_eq!(cache.lru[1], "example.net");
        assert_eq!(cache.lru[2], "example.org");

        cache.announce(
            "example.net",
            &DnsEntry::new("example.net", vec!["127.0.0.1"]),
        );
        assert_eq!(cache.values.len(), 3);
        assert_eq!(cache.lru.len(), 3);
        assert_eq!(cache.lru[0], "example.com");
        assert_eq!(cache.lru[1], "example.org");
        assert_eq!(cache.lru[2], "example.net");

        let _ = cache.resolve("example.com", ResolveType::Both);
        assert_eq!(cache.lru[0], "example.org");
        assert_eq!(cache.lru[1], "example.net");
        assert_eq!(cache.lru[2], "example.com");

        cache.announce("new.com", &DnsEntry::new("new.com", vec!["127.0.0.1"]));
        assert_eq!(cache.values.len(), 3);
        assert_eq!(cache.lru.len(), 3);
        assert_eq!(cache.lru[0], "example.net");
        assert_eq!(cache.lru[1], "example.com");
        assert_eq!(cache.lru[2], "new.com");
    }
}
