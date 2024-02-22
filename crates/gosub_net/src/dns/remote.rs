use crate::dns::{DnsEntry, DnsResolver, ResolveType};
use crate::errors::Error;
use core::str::FromStr;
use gosub_shared::types::Result;
use hickory_resolver::config::Protocol::Udp;
use hickory_resolver::config::{NameServerConfig, ResolverConfig, ResolverOpts};
use hickory_resolver::Resolver;
use log::trace;
use std::net::{IpAddr, SocketAddr};

pub struct RemoteResolver {
    hickory: Resolver,
}

impl DnsResolver for RemoteResolver {
    fn resolve(&mut self, domain: &str, resolve_type: ResolveType) -> Result<DnsEntry> {
        let mut entry = DnsEntry::new(domain, vec![]);

        let mut ip_types = Vec::new();
        match resolve_type {
            ResolveType::Ipv4 => {
                ip_types.push(ResolveType::Ipv4);
            }
            ResolveType::Ipv6 => {
                ip_types.push(ResolveType::Ipv6);
            }
            ResolveType::Both => {
                ip_types.push(ResolveType::Ipv6);
                ip_types.push(ResolveType::Ipv4);
            }
        }

        trace!("{domain}: resolving with {ip_types:?}");

        for ip_type in &ip_types {
            match *ip_type {
                ResolveType::Ipv4 => {
                    let e = self.hickory.ipv4_lookup(domain);
                    if e.is_err() {
                        continue;
                    }
                    e.unwrap().iter().for_each(|ip| {
                        trace!("{domain}: found ipv4 address {ip}");
                        entry
                            .ips
                            .push(IpAddr::from_str(ip.to_string().as_str()).unwrap());
                        entry.has_ipv4 = true;
                    });
                }
                ResolveType::Ipv6 => {
                    let e = self.hickory.ipv6_lookup(domain);
                    if e.is_err() {
                        continue;
                    }
                    e.unwrap().iter().for_each(|ip| {
                        trace!("{domain}: found ipv6 address {ip}");
                        entry
                            .ips
                            .push(IpAddr::from_str(ip.to_string().as_str()).unwrap());
                        entry.has_ipv6 = true;
                    });
                }
                ResolveType::Both => {}
            }
        }

        if !entry.has_ipv4 && !entry.has_ipv6 {
            return Err(Error::DnsNoIpAddressFound.into());
        }

        Ok(entry)
    }

    fn name(&self) -> &'static str {
        "remote resolver"
    }
}

impl RemoteResolver {
    /// Instantiates a new local override table
    pub fn new(dns_opts: RemoteResolverOptions) -> Self {
        // @todo: do something with the options
        let mut config = ResolverConfig::default();
        let mut opts = ResolverOpts::default();

        for nameserver in &dns_opts.nameservers {
            if let Ok(ip) = IpAddr::from_str(nameserver.as_str()) {
                config.add_name_server(NameServerConfig::new(SocketAddr::new(ip, 53), Udp));
                continue;
            }
        }
        opts.use_hosts_file = dns_opts.use_hosts_file;
        opts.timeout = std::time::Duration::from_secs(dns_opts.timeout as u64);
        opts.attempts = dns_opts.retries;

        Self {
            hickory: Resolver::new(config, opts).unwrap(),
        }
    }
}

/// Options for the remote resolver
pub struct RemoteResolverOptions {
    pub timeout: usize,
    pub retries: usize,
    pub use_hosts_file: bool,
    pub nameservers: Vec<String>,
}

impl Default for RemoteResolverOptions {
    fn default() -> Self {
        Self {
            timeout: 5,
            retries: 3,
            use_hosts_file: true,
            nameservers: vec![],
        }
    }
}
