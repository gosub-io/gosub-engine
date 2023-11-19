use crate::dns::{DnsEntry, DnsResolver, ResolveType};
use crate::types;
use core::str::FromStr;
use hickory_resolver::config::{ResolverConfig, ResolverOpts};
use hickory_resolver::Resolver;
use log::trace;
use std::net::IpAddr;

pub struct RemoteResolver {
    hickory: Resolver,
}

impl DnsResolver for RemoteResolver {
    fn resolve(
        &mut self,
        domain: &str,
        resolve_type: ResolveType,
    ) -> Result<DnsEntry, types::Error> {
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

        trace!("{}: resolving with {:?}", domain, ip_types);

        for ip_type in ip_types.iter() {
            match *ip_type {
                ResolveType::Ipv4 => {
                    let e = self.hickory.ipv4_lookup(domain);
                    if e.is_err() {
                        continue;
                    }
                    e.unwrap().iter().for_each(|ip| {
                        trace!("{}: found ipv4 address {}", domain, ip);
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
                        trace!("{}: found ipv6 address {}", domain, ip);
                        entry
                            .ips
                            .push(IpAddr::from_str(ip.to_string().as_str()).unwrap());
                        entry.has_ipv6 = true;
                    });
                }
                _ => {}
            }
        }

        if !entry.has_ipv4 && !entry.has_ipv6 {
            return Err(types::Error::DnsNoIpAddressFound);
        }

        Ok(entry)
    }

    fn name(&self) -> &'static str {
        "remote resolver"
    }
}

impl RemoteResolver {
    /// Instantiates a new local override table
    pub fn new(_opts: RemoteResolverOptions) -> RemoteResolver {
        // @todo: do something with the options
        let config = ResolverConfig::default();
        let opts = ResolverOpts::default();

        RemoteResolver {
            hickory: Resolver::new(config, opts).unwrap(),
        }
    }
}

/// Options for the remote resolver
pub struct RemoteResolverOptions {
    pub timeout: u32,
    pub retries: u32,
    pub nameservers: Vec<String>,
}

impl Default for RemoteResolverOptions {
    fn default() -> Self {
        RemoteResolverOptions {
            timeout: 5,
            retries: 3,
            nameservers: vec![],
        }
    }
}
