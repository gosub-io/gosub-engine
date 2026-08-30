//! Private-network protection: which destinations a page may make the engine
//! connect to.
//!
//! A page from the public internet must not be able to use the browser as a
//! foothold into the network it runs on - `http://169.254.169.254/` (cloud
//! metadata), `http://10.0.0.5/admin`, `http://localhost:9200/` - through an
//! `<img>`, a stylesheet or a script tag. That is the classic SSRF shape, made
//! worse by process isolation: the network process is the one component allowed
//! to open sockets, so it is exactly where a compromised renderer would aim.
//!
//! The policy is the one browsers converge on (Private Network Access): a
//! *subresource* request from a **public** document may not reach a **private**
//! address. Navigations are never restricted - the user typing `localhost` is
//! not an attack - and a document that itself came from the private network may
//! load its own neighbours.
//!
//! ## Deciding and connecting are one step
//!
//! "Is this URL allowed?" cannot be made safe for hostnames on its own: the
//! caller still connects, connecting resolves the name again, and the attacker
//! controls the second answer (DNS rebinding). So the decision is not a
//! pre-check but a property of the *connection*: a strict fetcher resolves
//! through [`StrictResolver`], which classifies every answer and refuses the
//! name if any is private, and gosub-sonar looks names up per connection and
//! per redirect hop through that resolver alone. There is no second lookup to
//! poison. IP literals never reach a resolver; [`literal_verdict`] classifies
//! them per hop through the fetcher's URL policy.
//!
//! The classification is deliberately wide: every range a renderer must never
//! reach, plus the alternate IPv4 spellings (`2130706433`, `0x7f000001`,
//! `127.1`) and IPv6 embeddings (NAT64, 6to4, IPv4-mapped) that naive filters
//! miss.

use gosub_sonar::{DnsError, DnsResolver, Resolving};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};
use url::Url;

/// How long a hostname's classification is remembered. Long enough that the
/// document's own address space is not re-resolved for every subresource;
/// short enough that a legitimately re-addressed host recovers.
const CLASSIFICATION_TTL: Duration = Duration::from_secs(300);

/// Classify an IP against the ranges that must never be reachable from a
/// public page. Returns the category name, or `None` if the address is public.
pub fn blocked_ip_reason(ip: IpAddr) -> Option<&'static str> {
    match ip {
        IpAddr::V4(v4) => blocked_v4(v4),
        IpAddr::V6(v6) => {
            // An IPv4-mapped address (::ffff:a.b.c.d) reaches an IPv4 host.
            if let Some(v4) = v6.to_ipv4_mapped() {
                return blocked_v4(v4);
            }
            let seg = v6.segments();
            if v6.is_loopback() {
                Some("IPv6 loopback (::1)")
            } else if v6.is_unspecified() {
                Some("IPv6 unspecified (::)")
            } else if seg[0] & 0xfe00 == 0xfc00 {
                Some("IPv6 unique-local (fc00::/7)")
            } else if seg[0] & 0xffc0 == 0xfe80 {
                Some("IPv6 link-local (fe80::/10)")
            } else if v6.is_multicast() {
                Some("IPv6 multicast")
            } else if seg[0] == 0x64 && seg[1] == 0xff9b && seg[2..6] == [0, 0, 0, 0] {
                // NAT64 (64:ff9b::/96): what gets reached is the embedded IPv4.
                blocked_v4(embedded_v4(seg))
            } else if seg[..6] == [0, 0, 0, 0, 0, 0] {
                // Deprecated IPv4-compatible (::a.b.c.d): same reach as the
                // embedded IPv4 on stacks that still honor it.
                blocked_v4(embedded_v4(seg))
            } else if seg[0] == 0x2002 {
                // 6to4 (2002:AABB:CCDD::): the IPv4 sits in the next 32 bits.
                let v4 = Ipv4Addr::new((seg[1] >> 8) as u8, seg[1] as u8, (seg[2] >> 8) as u8, seg[2] as u8);
                blocked_v4(v4)
            } else {
                None
            }
        }
    }
}

/// The IPv4 address in the low 32 bits of an IPv6 address (NAT64 and
/// IPv4-compatible embeddings).
fn embedded_v4(seg: [u16; 8]) -> Ipv4Addr {
    Ipv4Addr::new((seg[6] >> 8) as u8, seg[6] as u8, (seg[7] >> 8) as u8, seg[7] as u8)
}

fn blocked_v4(v4: Ipv4Addr) -> Option<&'static str> {
    let o = v4.octets();
    if v4.is_loopback() {
        Some("loopback (127.0.0.0/8)")
    } else if v4.is_private() {
        Some("private (10/8, 172.16/12, 192.168/16)")
    } else if v4.is_link_local() {
        Some("link-local 169.254.0.0/16 (cloud metadata)")
    } else if v4.is_unspecified() || o[0] == 0 {
        Some("\"this host\" (0.0.0.0/8)")
    } else if v4.is_broadcast() {
        Some("broadcast (255.255.255.255)")
    } else if o[0] == 100 && o[1] & 0xc0 == 64 {
        Some("shared/CGNAT (100.64.0.0/10)")
    } else if v4.is_multicast() {
        Some("IPv4 multicast (224.0.0.0/4)")
    } else if o[0] >= 240 {
        Some("reserved class E (240.0.0.0/4)")
    } else if o[0] == 192 && o[1] == 0 && o[2] == 0 {
        Some("IETF protocol assignments (192.0.0.0/24)")
    } else if o[0] == 192 && o[1] == 88 && o[2] == 99 {
        Some("6to4 relay anycast (192.88.99.0/24)")
    } else if o[0] == 198 && o[1] & 0xfe == 18 {
        Some("benchmarking (198.18.0.0/15)")
    } else if (o[0] == 192 && o[1] == 0 && o[2] == 2)
        || (o[0] == 198 && o[1] == 51 && o[2] == 100)
        || (o[0] == 203 && o[1] == 0 && o[2] == 113)
    {
        Some("documentation TEST-NET (192.0.2/24, 198.51.100/24, 203.0.113/24)")
    } else {
        None
    }
    // Deliberately NOT blocked: subnet-directed broadcast (x.y.z.255) - which
    // addresses are broadcasts depends on the local netmask, and refusing
    // every .255 would break legitimate public hosts.
}

/// Parse a host as an IP literal, accepting the alternate IPv4 encodings that
/// `inet_aton(3)` and browsers accept (a single decimal/octal/hex number, or
/// fewer than four dotted parts) - the encodings SSRF filters classically miss.
pub fn parse_ip_literal(host: &str) -> Option<IpAddr> {
    let host = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')).unwrap_or(host);
    // Strip an IPv6 zone id (`fe80::1%eth0`, or percent-encoded `%25eth0`) so a
    // scoped link-local literal is classified numerically.
    let host = host.split('%').next().unwrap_or(host);
    let host = host.trim_end_matches('.');
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Some(ip);
    }
    parse_ipv4_inet_aton(host).map(IpAddr::V4)
}

fn parse_ipv4_inet_aton(host: &str) -> Option<Ipv4Addr> {
    let parts: Vec<u32> = host.split('.').map(parse_c_integer).collect::<Option<_>>()?;
    // 1-4 parts; the final part fills all remaining low-order bytes.
    let value: u32 = match parts.as_slice() {
        [a] => *a,
        [a, b] if *a <= 0xff && *b <= 0x00ff_ffff => (a << 24) | b,
        [a, b, c] if *a <= 0xff && *b <= 0xff && *c <= 0xffff => (a << 24) | (b << 16) | c,
        [a, b, c, d] if [a, b, c, d].iter().all(|&&x| x <= 0xff) => (a << 24) | (b << 16) | (c << 8) | d,
        _ => return None,
    };
    Some(Ipv4Addr::from(value))
}

/// A C-style integer: `0x`/`0X` hex, a leading `0` octal, otherwise decimal.
fn parse_c_integer(s: &str) -> Option<u32> {
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).ok()
    } else if s.len() > 1 && s.starts_with('0') {
        u32::from_str_radix(&s[1..], 8).ok()
    } else {
        s.parse::<u32>().ok()
    }
}

/// Why a URL with an IP-literal host may not be fetched by a strict fetcher,
/// or `None` when it is a hostname (the resolver's business) or a public
/// literal. This is the per-hop URL policy; it also refuses non-HTTP schemes,
/// which a strict fetcher never has business with.
pub fn literal_verdict(url: &Url) -> Option<String> {
    if !matches!(url.scheme(), "http" | "https") {
        return Some(format!("scheme {}:// is not allowed for a subresource", url.scheme()));
    }
    let host = url.host_str()?;
    let ip = parse_ip_literal(host)?;
    blocked_ip_reason(ip).map(|category| format!("host {host} is {category} (private network policy)"))
}

/// Resolves through the system resolver and refuses any name with a private
/// answer - the whole name, not just the offending address: which answer the
/// OS would connect to is not this code's choice, so a name answering
/// `[1.2.3.4, 127.0.0.1]` is one round-robin away from loopback.
#[derive(Debug, Default)]
pub struct StrictResolver;

impl DnsResolver for StrictResolver {
    fn resolve(&self, host: &str) -> Resolving {
        let host = host.to_string();
        Box::pin(async move {
            let addrs = lookup(&host).await.map_err(|e| -> DnsError { e.into() })?;
            if addrs.is_empty() {
                return Err(format!("host {host} did not resolve").into());
            }
            for ip in &addrs {
                if let Some(category) = blocked_ip_reason(*ip) {
                    return Err(
                        format!("host {host} resolves to {ip}, which is {category} (private network policy)").into(),
                    );
                }
            }
            Ok(addrs.into_iter().map(|ip| SocketAddr::new(ip, 0)).collect())
        })
    }
}

async fn lookup(host: &str) -> std::io::Result<Vec<IpAddr>> {
    Ok(tokio::net::lookup_host((host, 0u16)).await?.map(|sa| sa.ip()).collect())
}

/// Where a URL's host lives, as the private-network policy sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressSpace {
    Public,
    /// Loopback, private, link-local, ... - anything in [`blocked_ip_reason`]'s
    /// ranges. A name with one private answer is private.
    Private,
}

const MAX_CACHED_HOSTS: usize = 4096;

/// Remembers which address space a host was classified into, so a document's
/// own host is not resolved again for every subresource it loads.
#[derive(Debug, Default)]
pub struct AddressSpaceCache {
    hosts: Mutex<HashMap<String, (Instant, AddressSpace)>>,
}

impl AddressSpaceCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// The address space of `url`'s host. IP literals are classified directly;
    /// names are resolved once (any private answer makes the name private) and
    /// remembered. A name that does not resolve counts as public: the policy
    /// then applies to what it loads, which is the safe direction.
    pub async fn classify(&self, url: &Url) -> AddressSpace {
        let Some(host) = url.host_str() else {
            return AddressSpace::Public;
        };
        if let Some(ip) = parse_ip_literal(host) {
            return space_of(std::iter::once(ip));
        }
        let key = cow_utils::CowUtils::cow_to_ascii_lowercase(host).into_owned();
        if let Some((seen, space)) = self.hosts.lock().get(&key) {
            if seen.elapsed() < CLASSIFICATION_TTL {
                return *space;
            }
        }
        let space = match lookup(&key).await {
            Ok(addrs) => space_of(addrs.into_iter()),
            Err(_) => AddressSpace::Public,
        };
        let mut hosts = self.hosts.lock();
        hosts.insert(key, (Instant::now(), space));
        // Expired entries are only ever skipped on lookup; sweep them here
        // when the table grows past what a session plausibly touches.
        if hosts.len() > MAX_CACHED_HOSTS {
            hosts.retain(|_, (seen, _)| seen.elapsed() < CLASSIFICATION_TTL);
            if hosts.len() > MAX_CACHED_HOSTS {
                hosts.clear();
            }
        }
        space
    }
}

fn space_of(mut addrs: impl Iterator<Item = IpAddr>) -> AddressSpace {
    if addrs.any(|ip| blocked_ip_reason(ip).is_some()) {
        AddressSpace::Private
    } else {
        AddressSpace::Public
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn blocks_internal_ranges_and_encoding_bypasses() {
        for u in [
            // Standard internal ranges (incl. 172.16/12).
            "http://127.0.0.1/",
            "http://10.0.0.5/",
            "http://172.16.0.1/",
            "http://172.31.255.9/",
            "http://192.168.1.1/",
            "http://169.254.169.254/",
            "http://0.0.0.0/",
            "http://100.64.0.1/",
            "http://255.255.255.255/",
            // Alternate IPv4 encodings for 127.0.0.1 (the URL parser normalizes
            // these to dotted quads; the literal parser handles them raw too).
            "http://2130706433/",
            "http://0x7f000001/",
            "http://017700000001/",
            "http://127.1/",
            // IPv6 internal, IPv4-mapped, scoped.
            "http://[::1]/",
            "http://[::ffff:169.254.169.254]/",
            "http://[fc00::1]/",
            "http://[fe80::1]/",
            // Multicast, class E, and the special-purpose IPv4 registry blocks.
            "http://224.0.0.1/",
            "http://239.255.255.250/",
            "http://240.0.0.1/",
            "http://192.0.0.5/",
            "http://192.88.99.1/",
            "http://198.18.0.1/",
            "http://198.19.255.1/",
            "http://192.0.2.1/",
            "http://198.51.100.7/",
            "http://203.0.113.9/",
            // IPv6 embeddings that reach internal IPv4: NAT64, IPv4-compatible, 6to4.
            "http://[64:ff9b::7f00:1]/",
            "http://[64:ff9b::a00:1]/",
            "http://[::127.0.0.1]/",
            "http://[2002:c0a8:0101::]/",
            "http://[2002:7f00:0001::]/",
            // Parser confusion: userinfo and trailing dot.
            "http://real.com@127.0.0.1/",
            "http://127.0.0.1.:80/",
            // Non-HTTP schemes are refused outright, whatever the host.
            "ftp://127.0.0.1/",
        ] {
            let verdict = literal_verdict(&url(u));
            assert!(verdict.is_some(), "should block {u}");
        }
    }

    #[test]
    fn allows_public_addresses_and_hostnames() {
        for u in [
            "http://93.184.216.34/",
            "http://example.com/",
            "http://8.8.8.8/",
            "http://172.32.0.1/",    // just outside 172.16/12
            "http://100.128.0.1/",   // just outside 100.64/10
            "http://223.255.255.1/", // just below multicast
            "http://198.20.0.1/",    // just outside benchmarking 198.18/15
            "http://[2606:2800:220:1::1]/",
            "http://[64:ff9b::808:808]/",  // NAT64 embedding a public v4 (8.8.8.8)
            "http://[2002:5db8:d822::1]/", // 6to4 embedding a public v4
        ] {
            assert_eq!(literal_verdict(&url(u)), None, "should allow {u}");
        }
    }

    #[test]
    fn alternate_ipv4_encodings_parse_to_loopback() {
        let loopback = ip("127.0.0.1");
        for h in [
            "2130706433",
            "0x7f000001",
            "017700000001",
            "127.1",
            "[::ffff:127.0.0.1]",
        ] {
            let parsed = parse_ip_literal(h).map(|ip| match ip {
                IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
                v4 => v4,
            });
            assert_eq!(parsed, Some(loopback), "{h}");
        }
        assert_eq!(parse_ip_literal("example.com"), None);
        assert_eq!(
            parse_ip_literal("[fe80::1%25eth0]").map(|ip| blocked_ip_reason(ip).is_some()),
            Some(true)
        );
    }

    #[tokio::test]
    async fn literals_classify_without_resolving() {
        let cache = AddressSpaceCache::new();
        assert_eq!(cache.classify(&url("http://10.1.2.3/")).await, AddressSpace::Private);
        assert_eq!(cache.classify(&url("http://[::1]:8080/")).await, AddressSpace::Private);
        assert_eq!(
            cache.classify(&url("http://93.184.216.34/")).await,
            AddressSpace::Public
        );
    }

    #[tokio::test]
    async fn loopback_names_are_private_and_strictly_refused() {
        // `localhost` resolves without any network; the system answers loopback.
        let cache = AddressSpaceCache::new();
        assert_eq!(cache.classify(&url("http://localhost/")).await, AddressSpace::Private);
        let err = StrictResolver
            .resolve("localhost")
            .await
            .expect_err("loopback must be refused");
        assert!(err.to_string().contains("private network policy"), "{err}");
    }

    /// Deterministic stand-in for a fuzz target: the literal parser must classify
    /// or reject any string without panicking - a parser panic in the one
    /// process allowed to open sockets is itself a bug.
    #[test]
    fn literal_parsing_never_panics_on_arbitrary_hosts() {
        let alpha = b"[]%:.0123456789abcdefABCDEFxX-";
        let mut s = 0xdead_beef_cafe_babeu64;
        for _ in 0..50_000 {
            let len = (xorshift(&mut s) % 40) as usize;
            let host: String = (0..len)
                .map(|_| alpha[(xorshift(&mut s) as usize) % alpha.len()] as char)
                .collect();
            let _ = parse_ip_literal(&host);
        }
    }

    fn xorshift(s: &mut u64) -> u64 {
        let mut x = *s;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *s = x;
        x
    }
}
