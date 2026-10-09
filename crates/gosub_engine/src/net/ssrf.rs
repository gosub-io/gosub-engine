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
//! The policy is the one browsers converge on (Local Network Access, formerly
//! Private Network Access), with its three address spaces: **public**,
//! **local** (the private ranges, link-local, CGNAT, ...) and **loopback**. A
//! *subresource* may only reach its document's space or a more public one: a
//! public document reaches neither the local network nor loopback, a document
//! from the local network may load its neighbours but not the machine's own
//! services. Navigations are never restricted - the user typing `localhost` is
//! not an attack.
//!
//! ## Deciding and connecting are one step
//!
//! "Is this URL allowed?" cannot be made safe for hostnames on its own: the
//! caller still connects, connecting resolves the name again, and the attacker
//! controls the second answer (DNS rebinding). So the decision is not a
//! pre-check but a property of the *connection*: a strict fetcher resolves
//! through [`StrictResolver`], which classifies every answer and refuses the
//! name if any is beyond its reach, and gosub-sonar looks names up per connection and
//! per redirect hop through that resolver alone. There is no second lookup to
//! poison. IP literals never reach a resolver; [`literal_verdict`] classifies
//! them per hop through the fetcher's URL policy.
//!
//! The other half is which space a document is in, and there the same rule
//! holds: a document is judged by the address its response actually came from
//! ([`space_of_response`], from the connection's peer as gosub-sonar reports it),
//! never by resolving its host again. A second lookup is one a rebinding DNS
//! server answers differently: public for the connection that served the page,
//! private for the question "where does this page live?".
//!
//! Where the spec leaves link-local (`169.254.0.0/16`, cloud metadata) is the
//! local space, so a document from the local network can reach it.
//!
//! The classification is deliberately wide: every range a renderer must never
//! reach, plus the alternate IPv4 spellings (`2130706433`, `0x7f000001`,
//! `127.1`) and IPv6 embeddings (NAT64, 6to4, IPv4-mapped) that naive filters
//! miss.

use gosub_sonar::{DnsError, DnsResolver, Resolving};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use url::Url;

/// Classify an IP against the ranges that must never be reachable from a
/// public page: the address space it is in and the category name, or `None`
/// if the address is public.
pub fn classify_ip(ip: IpAddr) -> Option<(AddressSpace, &'static str)> {
    use AddressSpace::{Local, Loopback};
    match ip {
        IpAddr::V4(v4) => blocked_v4(v4),
        IpAddr::V6(v6) => {
            // An IPv4-mapped address (::ffff:a.b.c.d) reaches an IPv4 host.
            if let Some(v4) = v6.to_ipv4_mapped() {
                return blocked_v4(v4);
            }
            let seg = v6.segments();
            if v6.is_loopback() {
                Some((Loopback, "IPv6 loopback (::1)"))
            } else if v6.is_unspecified() {
                Some((Loopback, "IPv6 unspecified (::)"))
            } else if seg[0] & 0xfe00 == 0xfc00 {
                Some((Local, "IPv6 unique-local (fc00::/7)"))
            } else if seg[0] & 0xffc0 == 0xfe80 {
                Some((Local, "IPv6 link-local (fe80::/10)"))
            } else if v6.is_multicast() {
                Some((Local, "IPv6 multicast"))
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

fn blocked_v4(v4: Ipv4Addr) -> Option<(AddressSpace, &'static str)> {
    use AddressSpace::{Local, Loopback};
    let o = v4.octets();
    if v4.is_loopback() {
        Some((Loopback, "loopback (127.0.0.0/8)"))
    } else if v4.is_private() {
        Some((Local, "private (10/8, 172.16/12, 192.168/16)"))
    } else if v4.is_link_local() {
        Some((Local, "link-local 169.254.0.0/16 (cloud metadata)"))
    } else if v4.is_unspecified() || o[0] == 0 {
        // Connecting to 0.0.0.0 reaches this host's own services.
        Some((Loopback, "\"this host\" (0.0.0.0/8)"))
    } else if v4.is_broadcast() {
        Some((Local, "broadcast (255.255.255.255)"))
    } else if o[0] == 100 && o[1] & 0xc0 == 64 {
        Some((Local, "shared/CGNAT (100.64.0.0/10)"))
    } else if v4.is_multicast() {
        Some((Local, "IPv4 multicast (224.0.0.0/4)"))
    } else if o[0] >= 240 {
        Some((Local, "reserved class E (240.0.0.0/4)"))
    } else if o[0] == 192 && o[1] == 0 && o[2] == 0 {
        Some((Local, "IETF protocol assignments (192.0.0.0/24)"))
    } else if o[0] == 192 && o[1] == 88 && o[2] == 99 {
        Some((Local, "6to4 relay anycast (192.88.99.0/24)"))
    } else if o[0] == 198 && o[1] & 0xfe == 18 {
        Some((Local, "benchmarking (198.18.0.0/15)"))
    } else if (o[0] == 192 && o[1] == 0 && o[2] == 2)
        || (o[0] == 198 && o[1] == 51 && o[2] == 100)
        || (o[0] == 203 && o[1] == 0 && o[2] == 113)
    {
        Some((
            Local,
            "documentation TEST-NET (192.0.2/24, 198.51.100/24, 203.0.113/24)",
        ))
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

/// Why a URL with an IP-literal host may not be fetched by a strict fetcher
/// serving a document in `reach`, or `None` when it is a hostname (the
/// resolver's business) or a literal `reach` may reach. This is the per-hop
/// URL policy; it also refuses non-HTTP schemes, which a strict fetcher never
/// has business with.
pub fn literal_verdict(url: &Url, reach: AddressSpace) -> Option<String> {
    if !matches!(url.scheme(), "http" | "https") {
        return Some(format!("scheme {}:// is not allowed for a subresource", url.scheme()));
    }
    let host = url.host_str()?;
    let ip = parse_ip_literal(host)?;
    beyond(ip, reach).map(|category| format!("host {host} is {category} (private network policy)"))
}

/// The category of `ip` when a document in `reach` may not reach it.
fn beyond(ip: IpAddr, reach: AddressSpace) -> Option<&'static str> {
    classify_ip(ip)
        .filter(|(space, _)| !reach.may_reach(*space))
        .map(|(_, category)| category)
}

/// Resolves through the system resolver and refuses any name with an answer
/// beyond `reach` - the whole name, not just the offending address: which
/// answer the OS would connect to is not this code's choice, so a name
/// answering `[1.2.3.4, 127.0.0.1]` is one round-robin away from loopback.
#[derive(Debug)]
pub struct StrictResolver {
    pub reach: AddressSpace,
}

impl DnsResolver for StrictResolver {
    fn resolve(&self, host: &str) -> Resolving {
        let host = host.to_string();
        let reach = self.reach;
        Box::pin(async move {
            let addrs = lookup(&host).await.map_err(|e| -> DnsError { e.into() })?;
            if addrs.is_empty() {
                return Err(format!("host {host} did not resolve").into());
            }
            for ip in &addrs {
                if let Some(category) = beyond(*ip, reach) {
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

/// Where a URL's host lives, as the private-network policy sees it: the
/// spec's IP address spaces, from the most public to the most private. A
/// document's space lifts the protection off what it loads, so anything less
/// than certainty earns it public.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AddressSpace {
    Public,
    /// The private ranges, link-local, CGNAT, ... - every range in
    /// [`classify_ip`] that is not loopback.
    Local,
    /// This machine: `127.0.0.0/8`, `::1`, and `0.0.0.0`, which reaches it too.
    Loopback,
}

impl AddressSpace {
    /// Whether a document in this space may load from `destination`: its own
    /// space or a more public one.
    pub fn may_reach(self, destination: AddressSpace) -> bool {
        destination.rank() <= self.rank()
    }

    fn rank(self) -> u8 {
        match self {
            AddressSpace::Public => 0,
            AddressSpace::Local => 1,
            AddressSpace::Loopback => 2,
        }
    }
}

/// The address space of a response served for `final_url`, from `peer`, the
/// address its connection reached (see `FetchResultMeta::peer_addr`). Without
/// one - a proxied or synthetic response - only an IP-literal host can be
/// placed; a name is public.
pub fn space_of_response(final_url: &Url, peer: Option<SocketAddr>) -> AddressSpace {
    if let Some(peer) = peer {
        return space_of(std::iter::once(peer.ip()));
    }
    match final_url.host_str().and_then(parse_ip_literal) {
        Some(ip) => space_of(std::iter::once(ip)),
        None => AddressSpace::Public,
    }
}

/// The most public space among `addrs`; public when there are none.
fn space_of(addrs: impl Iterator<Item = IpAddr>) -> AddressSpace {
    addrs
        .map(|ip| classify_ip(ip).map_or(AddressSpace::Public, |(space, _)| space))
        .min_by_key(|space| space.rank())
        .unwrap_or(AddressSpace::Public)
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
            let verdict = literal_verdict(&url(u), AddressSpace::Public);
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
            assert_eq!(literal_verdict(&url(u), AddressSpace::Public), None, "should allow {u}");
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
            parse_ip_literal("[fe80::1%25eth0]")
                .and_then(classify_ip)
                .map(|(space, _)| space),
            Some(AddressSpace::Local)
        );
    }

    /// A mixed answer places a document in its most public space: anything
    /// else would lift protection off its subresources on the strength of one
    /// private record.
    #[test]
    fn the_most_public_answer_places_a_document() {
        use AddressSpace::{Local, Loopback, Public};
        assert_eq!(space_of([ip("127.0.0.1"), ip("::1")].into_iter()), Loopback);
        assert_eq!(space_of([ip("127.0.0.1"), ip("10.0.0.1")].into_iter()), Local);
        assert_eq!(space_of([ip("10.0.0.1"), ip("93.184.216.34")].into_iter()), Public);
        assert_eq!(space_of([ip("93.184.216.34"), ip("192.168.1.1")].into_iter()), Public);
        assert_eq!(space_of(std::iter::empty()), Public);
    }

    /// Loopback is its own space, in every spelling that reaches it; the rest
    /// of the non-public ranges are local.
    #[test]
    fn loopback_is_set_apart_from_the_local_network() {
        let space = |h: &str| parse_ip_literal(h).and_then(classify_ip).map(|(space, _)| space);
        for h in [
            "127.0.0.1",
            "127.255.0.9",
            "0.0.0.0",
            "2130706433",
            "[::1]",
            "[::]",
            "[::ffff:127.0.0.1]",
            "[64:ff9b::7f00:1]",
            "[::127.0.0.1]",
            "[2002:7f00:0001::]",
        ] {
            assert_eq!(space(h), Some(AddressSpace::Loopback), "{h}");
        }
        for h in [
            "10.0.0.5",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "[fc00::1]",
            "[fe80::1]",
            "[::ffff:192.168.1.1]",
            "[2002:c0a8:0101::]",
        ] {
            assert_eq!(space(h), Some(AddressSpace::Local), "{h}");
        }
        assert_eq!(space("93.184.216.34"), None);
    }

    /// A document reaches its own space and the more public ones, never a more
    /// private one: a page from the local network loads its neighbours but not
    /// this machine's services.
    #[test]
    fn a_document_reaches_its_own_space_and_more_public_ones() {
        use AddressSpace::{Local, Loopback, Public};
        let refused = |u: &str, reach| literal_verdict(&url(u), reach).is_some();
        for u in [
            "http://127.0.0.1:9200/",
            "http://0.0.0.0/",
            "http://[::1]/",
            "http://[::ffff:127.0.0.1]/",
        ] {
            assert!(refused(u, Public), "{u} from a public document");
            assert!(refused(u, Local), "{u} from a local document");
            assert!(!refused(u, Loopback), "{u} from a loopback document");
        }
        for u in [
            "http://10.0.0.5/",
            "http://192.168.1.1/",
            "http://169.254.169.254/",
            "http://[fc00::1]/",
        ] {
            assert!(refused(u, Public), "{u} from a public document");
            assert!(!refused(u, Local), "{u} from a local document");
        }
        assert!(!refused("http://93.184.216.34/", Local));
        assert!(refused("ftp://10.0.0.5/", Local), "a strict fetcher is http(s) only");
        assert!(Loopback.may_reach(Public) && Local.may_reach(Public) && !Public.may_reach(Local));
    }

    /// A response is placed by the address it came from, whatever its host
    /// would resolve to now; without one, only a literal host is placed.
    #[test]
    fn a_response_is_placed_by_its_peer() {
        let peer = |s: &str| Some(SocketAddr::new(ip(s), 80));
        let named = url("http://rebind.example/");
        assert_eq!(space_of_response(&named, peer("127.0.0.1")), AddressSpace::Loopback);
        assert_eq!(space_of_response(&named, peer("::ffff:10.0.0.1")), AddressSpace::Local);
        assert_eq!(space_of_response(&named, peer("93.184.216.34")), AddressSpace::Public);
        // No peer (proxied, synthetic): a name is public, a literal is what it says.
        assert_eq!(space_of_response(&named, None), AddressSpace::Public);
        assert_eq!(space_of_response(&url("http://localhost/"), None), AddressSpace::Public);
        assert_eq!(space_of_response(&url("http://10.1.2.3/"), None), AddressSpace::Local);
        assert_eq!(
            space_of_response(&url("http://[::1]:8080/"), None),
            AddressSpace::Loopback
        );
        assert_eq!(
            space_of_response(&url("http://93.184.216.34/"), None),
            AddressSpace::Public
        );
        assert_eq!(
            space_of_response(&url("file:///tmp/a.html"), None),
            AddressSpace::Public
        );
    }

    #[tokio::test]
    async fn loopback_names_are_strictly_refused() {
        for reach in [AddressSpace::Public, AddressSpace::Local] {
            let err = StrictResolver { reach }
                .resolve("localhost")
                .await
                .expect_err("loopback must be refused");
            assert!(err.to_string().contains("private network policy"), "{err}");
        }
        let served = StrictResolver {
            reach: AddressSpace::Loopback,
        }
        .resolve("localhost")
        .await;
        assert!(served.is_ok(), "a loopback document reaches loopback: {served:?}");
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
