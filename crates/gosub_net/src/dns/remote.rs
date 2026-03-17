use crate::dns::{DnsEntry, DnsResolver, ResolveType};
use crate::errors::Error;
use gosub_shared::types::Result;
use hickory_resolver::config::{LookupIpStrategy, NameServerConfig, ResolveHosts, ResolverConfig, ResolverOpts};
use hickory_resolver::name_server::TokioConnectionProvider;
use hickory_resolver::proto::xfer::Protocol;
use hickory_resolver::TokioResolver;
use log::trace;
use std::net::{IpAddr, SocketAddr};
use tokio::runtime::Builder;

pub struct RemoteResolver {
    hickory: TokioResolver,
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

        let mut opts = self.hickory.options().clone();

        for ip_type in &ip_types {
            match *ip_type {
                ResolveType::Ipv4 => {
                    opts.ip_strategy = LookupIpStrategy::Ipv4Only;
                    let e = self.lookup(domain, opts.clone());
                    if e.is_err() {
                        continue;
                    }
                    e.unwrap().iter().for_each(|ip| {
                        trace!("{domain}: found ipv4 address {ip}");
                        entry.ips.push(ip);
                        entry.has_ipv4 = true;
                    });
                }
                ResolveType::Ipv6 => {
                    opts.ip_strategy = LookupIpStrategy::Ipv6Only;
                    let e = self.lookup(domain, opts.clone());
                    if e.is_err() {
                        continue;
                    }
                    e.unwrap().iter().for_each(|ip| {
                        trace!("{domain}: found ipv6 address {ip}");
                        entry.ips.push(ip);
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
    fn lookup(&self, domain: &str, opts: ResolverOpts) -> Result<hickory_resolver::lookup_ip::LookupIp> {
        let resolver =
            TokioResolver::builder_with_config(self.hickory.config().clone(), TokioConnectionProvider::default())
                .with_options(opts)
                .build();
        let rt = Builder::new_current_thread().enable_all().build()?;
        Ok(rt.block_on(resolver.lookup_ip(domain))?)
    }

    /// Instantiates a new local override table
    pub fn new(dns_opts: RemoteResolverOptions) -> Self {
        let mut opts = ResolverOpts::default();
        let has_custom_nameservers = !dns_opts.nameservers.is_empty();
        let mut config = ResolverConfig::default();

        for nameserver in &dns_opts.nameservers {
            if let Ok(ip) = nameserver.parse::<IpAddr>() {
                config.add_name_server(NameServerConfig::new(SocketAddr::new(ip, 53), Protocol::Udp));
            }
        }
        opts.use_hosts_file = if dns_opts.use_hosts_file {
            ResolveHosts::Always
        } else {
            ResolveHosts::Never
        };
        opts.timeout = std::time::Duration::from_secs(dns_opts.timeout as u64);
        opts.attempts = dns_opts.retries;

        let builder = if has_custom_nameservers {
            TokioResolver::builder_with_config(config, TokioConnectionProvider::default())
        } else {
            TokioResolver::builder_tokio().unwrap()
        };

        Self {
            hickory: builder.with_options(opts).build(),
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
