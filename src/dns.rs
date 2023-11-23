mod cache;
mod local;
mod remote;

use crate::types;
use crate::types::Result;
use core::str::FromStr;
use derive_more::Display;
use log::{debug, info};
use std::net::IpAddr;
use std::time::{SystemTime, UNIX_EPOCH};

/// A DNS entry is a mapping of a domain to zero or more IP address mapping
#[derive(Default, Clone, Debug, PartialEq)]
struct DnsEntry {
    // domain name
    domain: String,
    // // Ip type that is stored in this entry (could be Ipv4, IPv6 or Both)
    // ip_type:   ResolveType,
    // List of addresses for this domain
    ips: Vec<IpAddr>,

    /// True when the ips list has ipv4 addresses
    has_ipv4: bool,
    /// True when the ips list has ipv6 addresses
    has_ipv6: bool,

    // Internal iterator pointer
    iter: usize,
    /// expiry time after epoch
    expires: u64,
}

impl DnsEntry {
    /// Instantiate a new domain name entry with set of ips
    pub(crate) fn new(domain: &str, ips: Vec<&str>) -> DnsEntry {
        let mut entry = DnsEntry {
            domain: domain.to_string(),
            ..Default::default()
        };

        for ip in ips {
            if let Ok(ip) = IpAddr::from_str(ip) {
                if ip.is_ipv4() {
                    entry.has_ipv4 = true;
                }
                if ip.is_ipv6() {
                    entry.has_ipv6 = true;
                }
                entry.ips.push(ip);
            }
        }

        entry
    }

    /// Returns true if the dns entry has expired
    pub fn expired(&self) -> bool {
        self.expires
            < SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
    }

    fn ipv4(&self) -> Vec<IpAddr> {
        self.ips.iter().filter(|x| x.is_ipv4()).copied().collect()
    }

    fn ipv6(&self) -> Vec<IpAddr> {
        self.ips.iter().filter(|x| x.is_ipv6()).copied().collect()
    }

    fn iter(&self) -> impl Iterator<Item = &IpAddr> {
        self.ips.iter()
    }
}

/// Type of DNS resolution
#[derive(Display, Debug, PartialEq, Clone)]
pub enum ResolveType {
    /// Only resolve IPV4 addresses (A)
    Ipv4,
    /// Only resolve IPV6 addresses (AAAA)
    Ipv6,
    /// Resolve both IPV4 and IPV6 addresses
    Both,
}

trait DnsResolver {
    /// Resolves a domain name for a given resolver_type
    fn resolve(&mut self, domain: &str, resolve_type: ResolveType) -> Result<DnsEntry>;
    /// Announces the resolved dns entry for the domain to a resolver
    fn announce(&mut self, _domain: &str, _entry: &DnsEntry) {}
    // name for debugging purposes
    fn name(&self) -> &'static str;
}

trait DnsCache {
    /// Flush all domains
    fn flush_all(&mut self);
    /// Flush a single domain
    fn flush_entry(&mut self, domain: &str);
}

struct Dns {
    resolvers: Vec<Box<dyn DnsResolver>>,
}

impl Dns {
    pub fn new() -> Self {
        let mut resolvers: Vec<Box<dyn DnsResolver>> = vec![];
        resolvers.push(Box::new(cache::CacheResolver::new(1000)));
        resolvers.push(Box::new(local::LocalTableResolver::new()));

        let opts = remote::RemoteResolverOptions::default();
        resolvers.push(Box::new(remote::RemoteResolver::new(opts)));

        Dns { resolvers }
    }

    /// Resolves a domain name to a set of IP addresses based on the resolve_type.
    /// It can resolve either Ipv4, ipv6 or both addresses.
    ///
    /// Each request will be resolved by the resolvers in the order they are added.
    /// The first resolver is usually the cache resolver, which caches any entries (according to their TTL)
    /// The second resolver is usually the local table resolver, which resolves any local overrides
    /// The third resolver is usually the remote resolver, which resolves any remote entries by querying external DNS server(s)
    ///
    pub fn resolve(&mut self, domain: &str, resolve_type: ResolveType) -> Result<DnsEntry> {
        let mut entry = None;

        info!("Resolving {} for {:?}", domain, resolve_type);

        for resolver in self.resolvers.iter_mut() {
            debug!("Trying resolver: {}", resolver.name());

            if let Ok(e) = resolver.resolve(domain, resolve_type.clone()) {
                debug!("Found entry {:?}", e);
                entry = Some(e);
                break;
            }
        }

        if entry.is_none() {
            return Err(types::Error::DnsDomainNotFound);
        }

        // Iterate all resolvers and add to all cache systems (normally, this is only the first resolver)
        for resolver in self.resolvers.iter_mut() {
            resolver.announce(domain, &entry.clone().unwrap().clone());
        }

        Ok(entry.unwrap().clone())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use simple_logger::SimpleLogger;
    use std::time::Instant;

    #[test]
    fn resolver() {
        // Add simple logger, if not possible, that's fine too
        let _ = SimpleLogger::new().init();

        let mut dns = Dns::new();

        let now = Instant::now();
        let e = dns.resolve("example.org", ResolveType::Ipv4).unwrap();
        let elapsed_time = now.elapsed();
        e.ipv4().iter().for_each(|x| println!("ipv4: {}", x));
        println!("Took {} microseconds.", elapsed_time.as_micros());

        let now = Instant::now();
        let e = dns.resolve("example.org", ResolveType::Ipv6).unwrap();
        let elapsed_time = now.elapsed();
        e.ipv6().iter().for_each(|x| println!("ipv6: {}", x));
        println!("Took {} microseconds.", elapsed_time.as_micros());

        let now = Instant::now();
        let e = dns.resolve("example.org", ResolveType::Ipv4).unwrap();
        let elapsed_time = now.elapsed();
        e.ipv4().iter().for_each(|x| println!("ipv4: {}", x));
        println!("Took {} microseconds.", elapsed_time.as_micros());

        let now = Instant::now();
        let e = dns.resolve("example.org", ResolveType::Both).unwrap();
        let elapsed_time = now.elapsed();
        e.ipv4().iter().for_each(|x| println!("ipv4: {}", x));
        e.ipv6().iter().for_each(|x| println!("ipv6: {}", x));
        println!("Took {} microseconds.", elapsed_time.as_micros());
    }
}
