use crate::dns::{DnsEntry, DnsResolver, ResolveType};
use crate::types;
use core::str::FromStr;
use hickory_resolver::config::Protocol::Udp;
use hickory_resolver::config::{NameServerConfig, ResolverConfig, ResolverOpts};
use hickory_resolver::Resolver;
use log::trace;
use std::net::{IpAddr, SocketAddr};

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
    pub fn new(dns_opts: RemoteResolverOptions) -> RemoteResolver {
        // @todo: do something with the options
        let mut config = ResolverConfig::default();
        let mut opts = ResolverOpts::default();

        for nameserver in dns_opts.nameservers.iter() {
            if let Ok(ip) = IpAddr::from_str(nameserver.as_str()) {
                config.add_name_server(NameServerConfig::new(SocketAddr::new(ip, 53), Udp));
                continue;
            }
        }
        opts.use_hosts_file = dns_opts.use_hosts_file;
        opts.timeout = std::time::Duration::from_secs(dns_opts.timeout as u64);
        opts.attempts = dns_opts.retries;

        RemoteResolver {
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
        RemoteResolverOptions {
            timeout: 5,
            retries: 3,
            use_hosts_file: true,
            nameservers: vec![],
        }
    }
}
