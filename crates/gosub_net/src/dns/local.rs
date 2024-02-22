use crate::dns::{DnsCache, DnsEntry, DnsResolver, ResolveType};
use crate::errors::Error;
use core::fmt;
use domain_lookup_tree::DomainLookupTree;
use gosub_shared::types::Result;
use log::trace;
use std::collections::HashMap;

/// Local override table that can be used instead of using /etc/hosts or similar 3rd party dns system.
pub struct LocalTableResolver {
    /// Entries in the local override table.
    entries: HashMap<String, DnsEntry>,
    /// Domaintree is a hierarchical lookup tree for quick scanning of (wildcard) domains
    tree: DomainLookupTree,
}

impl Default for LocalTableResolver {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            tree: DomainLookupTree::new(),
        }
    }
}

impl DnsResolver for LocalTableResolver {
    fn resolve(&mut self, domain: &str, _resolve_type: ResolveType) -> Result<DnsEntry> {
        let Some(domain_entry) = self.tree.lookup(domain) else {
            trace!("{domain}: not found in local table");
            return Err(Error::DnsDomainNotFound.into());
        };

        trace!("{domain_entry}: found in local tree");

        // domain_entry could be "com" if you ask for just "com" and it's part of the tree (it normally is). So in
        // that case we still have to check if the domain is actually in the entries list.
        if let Some(entry) = self.entries.get(&domain_entry) {
            return Ok(entry.clone());
        }

        trace!("{domain}: not found in local table");
        Err(Error::DnsDomainNotFound.into())
    }

    fn name(&self) -> &'static str {
        "local table resolver"
    }
}

impl DnsCache for LocalTableResolver {
    fn flush_all(&mut self) {
        // flushing the local table means reloading the entries
        self.reload_table_entries();
    }

    fn flush_entry(&mut self, domain: &str) {
        self.reload_table_entry(domain);
    }
}

impl LocalTableResolver {
    /// Instantiates a new local override table
    #[must_use]
    pub fn new() -> Self {
        let mut table = Self {
            entries: HashMap::new(),
            tree: DomainLookupTree::new(),
        };

        table.reload_table_entries();
        table
    }

    /// Helper function to add an entry to the local override table. It will figure out which
    /// elements are ipv4 and ipv6 and add them accordingly
    #[allow(dead_code)]
    pub fn add_entry(&mut self, domain: &str, ips: Vec<&str>) {
        let entry = DnsEntry::new(domain, ips);

        self.entries.insert(domain.to_string(), entry);
        self.tree.insert(domain);
    }

    /// Regenerates the new entries table
    pub fn reload_table_entries(&mut self) {
        // @todo: this should reload all table entries from the configuration into the self.entries list
    }

    pub fn reload_table_entry(&mut self, _domain: &str) {
        // @todo: this should reload a single entry from the configuration into the self.entries
    }
}

impl fmt::Debug for LocalTableResolver {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "local table resolver")
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use core::str::FromStr;
    use std::net::IpAddr;

    #[test]
    fn test_local_override() {
        let mut table = LocalTableResolver::new();

        table.add_entry("example.com", vec!["1.2.3.4"]);
        table.add_entry("foo.example.com", vec!["2.3.4.5"]);
        table.add_entry(".wildcard.com", vec!["6.6.6.6"]);
        table.add_entry("specific.wildcard.com", vec!["8.8.8.8"]);
        table.add_entry("ipv6.com", vec!["2002::1", "2002::2", "200.200.200.200"]);

        // Simple resolve
        let e = table.resolve("example.com", ResolveType::Ipv4).unwrap();
        assert_eq!(
            &IpAddr::from_str("1.2.3.4").unwrap(),
            e.ips.first().unwrap()
        );
        assert!(table.resolve("xample.com", ResolveType::Ipv4).is_err());
        assert!(table.resolve("com", ResolveType::Ipv4).is_err());
        assert!(table.resolve("example", ResolveType::Ipv4).is_err());

        // Wildcard
        let e = table
            .resolve("specific.wildcard.com", ResolveType::Ipv4)
            .unwrap();
        assert_eq!(
            &IpAddr::from_str("8.8.8.8").unwrap(),
            e.ips.first().unwrap()
        );
        let e = table
            .resolve("something.wildcard.com", ResolveType::Ipv4)
            .unwrap();
        assert_eq!(
            &IpAddr::from_str("6.6.6.6").unwrap(),
            e.ips.first().unwrap()
        );
        let e = table
            .resolve("foobar.wildcard.com", ResolveType::Ipv4)
            .unwrap();
        assert_eq!(
            &IpAddr::from_str("6.6.6.6").unwrap(),
            e.ips.first().unwrap()
        );
        assert!(table.resolve("foo.custom.com", ResolveType::Ipv4).is_err());
        assert!(table
            .resolve("too.specific.wildcard.com", ResolveType::Ipv4)
            .is_err());
        assert!(table.resolve("custom.com", ResolveType::Ipv4).is_err());

        // round robin on both ipv4 and ipv6
        let e = table.resolve("ipv6.com", ResolveType::Ipv4).unwrap();
        assert_eq!(3, e.ips.len());
        assert_eq!(1, e.ipv4().len());
        assert_eq!(2, e.ipv6().len());
    }
}
