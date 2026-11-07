//! The SOCKS proxy an engine protocol connects through, and the client side of its `CONNECT`:
//! SOCKS5 (RFC 1928) with username/password (RFC 1929), and SOCKS4a.
//!
//! Which proxy is sonar's environment rules for http(s) (`ProxyConfig::System`), mirrored here
//! because sonar keeps its matcher to itself (`ProxyRoutes`, crate-private):
//! - only `ALL_PROXY` can apply: sonar scopes `HTTP_PROXY` and `HTTPS_PROXY` to http and https
//!   URLs. So the httpoxy rule (`HTTP_PROXY` ignored under CGI) never comes into it;
//! - the uppercase name wins over the lowercase one, and a value empty after trimming is unset;
//! - a value without a scheme is an http proxy;
//! - `NO_PROXY` is read as sonar reads it (`ProxyRule::no_proxy`): domains with their
//!   subdomains, IP addresses, CIDR blocks, `*` for everything, `[v6]`, a trailing dot, and
//!   `host:port` for that port only, the port being the protocol's when the URL names none.
//!
//! Only a SOCKS proxy is used. An http proxy cannot carry these protocols, so with one set they
//! connect directly, as they do with a value that is no usable proxy URL (sonar skips those
//! with a warning too). The operating system's proxy settings, which sonar also reads on macOS
//! and Windows, are not read here.
//!
//! The proxy resolves the host, whatever the URL's scheme says: `socks5://` is taken as
//! `socks5h://` and `socks4://` as `socks4a://`. curl and reqwest resolve locally for the plain
//! forms, but a lookup here tells the network what the user reaches, which is what a SOCKS
//! proxy is mostly set for. A SOCKS4 server without the 4a extension then refuses names.

use crate::net::ssrf::parse_ip_literal;
use cow_utils::CowUtils;
use percent_encoding::percent_decode_str;
use std::io::{Error, ErrorKind};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::LazyLock;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use url::{Host, Url};

/// The port of a proxy URL that names none, as for curl and reqwest.
const DEFAULT_PORT: u16 = 1080;

/// The process environment's proxy, read on first use and kept, as sonar reads it once when
/// its fetcher is built. Under the network process this is that process's environment, which
/// the spawner keeps proxy variables out of (`gosub_sandbox`'s `ENV_KEPT`), as it does for
/// sonar's http there: connections from that process are direct.
pub static FROM_ENV: LazyLock<Proxies> = LazyLock::new(|| Proxies::from_env(|name| std::env::var(name).ok()));

/// Which SOCKS the proxy speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Version {
    Socks4a,
    Socks5,
}

/// A SOCKS proxy.
#[derive(Clone, PartialEq, Eq)]
pub struct Proxy {
    pub version: Version,
    /// A name, or an IP address without brackets.
    pub host: String,
    pub port: u16,
    /// SOCKS5 username/password, or the SOCKS4 user id (the password unused).
    pub username: Option<String>,
    pub password: Option<String>,
}

impl std::fmt::Debug for Proxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Proxy")
            .field("version", &self.version)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "..."))
            .finish()
    }
}

impl std::fmt::Display for Proxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.host.parse::<IpAddr>() {
            Ok(IpAddr::V6(ip)) => write!(f, "[{ip}]:{}", self.port),
            _ => write!(f, "{}:{}", self.host, self.port),
        }
    }
}

impl Proxy {
    /// The SOCKS proxy `value` names, `None` for an http(s) proxy, or why it names none.
    fn parse(value: &str) -> Result<Option<Proxy>, String> {
        let with_scheme;
        let value = if value.contains("://") {
            value
        } else {
            with_scheme = format!("http://{value}");
            &with_scheme
        };
        let url = Url::parse(value).map_err(|e| format!("{value:?}: {e}"))?;
        let version = match url.scheme() {
            "socks5" | "socks5h" => Version::Socks5,
            "socks4" | "socks4a" => Version::Socks4a,
            "http" | "https" => return Ok(None),
            other => return Err(format!("{value:?}: {other} is no proxy scheme")),
        };
        let host = match url.host() {
            Some(Host::Domain(d)) if !d.is_empty() => d.to_string(),
            Some(Host::Ipv4(ip)) => ip.to_string(),
            Some(Host::Ipv6(ip)) => ip.to_string(),
            _ => return Err(format!("{value:?} names no host")),
        };
        let decoded = |s: &str| percent_decode_str(s).decode_utf8_lossy().into_owned();
        Ok(Some(Proxy {
            version,
            host,
            port: url.port().unwrap_or(DEFAULT_PORT),
            username: Some(decoded(url.username())).filter(|u| !u.is_empty()),
            password: url.password().map(decoded),
        }))
    }
}

/// The proxy the environment names for engine protocols, and the hosts that bypass it.
#[derive(Debug, Clone, Default)]
pub struct Proxies {
    all: Option<Proxy>,
    bypass: Vec<Bypass>,
}

impl Proxies {
    /// No proxy: every connection direct.
    pub fn none() -> Self {
        Self::default()
    }

    /// The proxy the variables `var` answers name, as sonar's `env_rules` reads them.
    pub fn from_env(var: impl Fn(&str) -> Option<String>) -> Self {
        let first = |names: &[&str]| {
            names
                .iter()
                .filter_map(|name| var(name))
                .map(|value| value.trim().to_string())
                .find(|value| !value.is_empty())
        };
        let all = first(&["ALL_PROXY", "all_proxy"]).and_then(|value| match Proxy::parse(&value) {
            Ok(Some(proxy)) => Some(proxy),
            Ok(None) => {
                log::info!(
                    "ALL_PROXY is an http proxy, which cannot carry gopher, finger or gemini: they connect directly"
                );
                None
            }
            Err(e) => {
                log::warn!("ignoring the proxy from the environment: {e}");
                None
            }
        });
        Proxies {
            all,
            bypass: first(&["NO_PROXY", "no_proxy"])
                .map(|list| bypass_entries(&list))
                .unwrap_or_default(),
        }
    }

    /// The proxy a connection to `host` on `port` goes through, `None` for a direct one.
    pub fn for_target(&self, host: &str, port: u16) -> Option<&Proxy> {
        let proxy = self.all.as_ref()?;
        let host = normalize_host(host);
        let ip = host.parse::<IpAddr>().ok();
        let bypassed = self.bypass.iter().any(|entry| match entry {
            Bypass::Everything => true,
            Bypass::Ip(net, prefix) => ip.is_some_and(|ip| in_net(ip, *net, *prefix)),
            Bypass::Name(name) => ip.is_none() && within(&host, name),
            Bypass::Port(name, entry_port) => *entry_port == port && within(&host, name),
        });
        (!bypassed).then_some(proxy)
    }
}

/// One `NO_PROXY` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Bypass {
    /// `*`.
    Everything,
    /// An address (a full-length prefix) or a CIDR block.
    Ip(IpAddr, u8),
    /// A name and its subdomains.
    Name(String),
    /// A name or address, and its subdomains, on one port.
    Port(String, u16),
}

/// The entries of a `NO_PROXY` list, split as sonar's `bypass_entries` splits it and matched
/// as hyper-util's `NoProxy` matches what that leaves.
fn bypass_entries(list: &str) -> Vec<Bypass> {
    list.split(',')
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .filter_map(|entry| {
            if entry == "*" {
                return Some(Bypass::Everything);
            }
            if let Some((ip, prefix)) = entry.split_once('/') {
                // Not a block is a name no host has.
                let ip: IpAddr = ip.parse().ok()?;
                let prefix: u8 = prefix.parse().ok()?;
                let bits = if ip.is_ipv4() { 32 } else { 128 };
                return (prefix <= bits).then_some(Bypass::Ip(ip, prefix));
            }
            // `[v6]` or `[v6]:port`.
            if let Some(rest) = entry.strip_prefix('[') {
                if let Some((host, after)) = rest.split_once(']') {
                    let port = after.strip_prefix(':').and_then(|p| p.parse().ok());
                    return Some(host_entry(normalize_host(host), port));
                }
            }
            // `host:port`; a bare IPv6 address has more than one colon and no port.
            if let Some((host, port)) = entry.split_once(':') {
                if !host.contains(':') && !port.contains(':') {
                    if let Ok(port) = port.parse() {
                        return Some(host_entry(normalize_host(host), Some(port)));
                    }
                }
            }
            Some(host_entry(normalize_host(entry), None))
        })
        .collect()
}

fn host_entry(host: String, port: Option<u16>) -> Bypass {
    match (port, host.parse::<IpAddr>()) {
        (Some(port), _) => Bypass::Port(host, port),
        (None, Ok(ip)) => Bypass::Ip(ip, if ip.is_ipv4() { 32 } else { 128 }),
        (None, Err(_)) => Bypass::Name(host),
    }
}

/// A host as entries and targets are compared: lowercase, without IPv6 brackets or a trailing
/// dot.
fn normalize_host(host: &str) -> String {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.strip_suffix('.')
        .unwrap_or(host)
        .cow_to_ascii_lowercase()
        .into_owned()
}

/// Whether `host` is `name` or a subdomain of it. A leading dot on `name` changes nothing.
fn within(host: &str, name: &str) -> bool {
    let name = name.strip_prefix('.').unwrap_or(name);
    !name.is_empty() && (host == name || host.ends_with(&format!(".{name}")))
}

/// Whether `ip` is in the block `net/prefix`.
fn in_net(ip: IpAddr, net: IpAddr, prefix: u8) -> bool {
    let bits = |ip: IpAddr| match ip {
        IpAddr::V4(v4) => (u128::from(u32::from(v4)), 32u32),
        IpAddr::V6(v6) => (u128::from(v6), 128u32),
    };
    let ((ip, len), (net, net_len)) = (bits(ip), bits(net));
    if len != net_len {
        return false;
    }
    let shift = len - u32::from(prefix);
    shift >= len || (ip >> shift) == (net >> shift)
}

/// A connection to `host` on `port` through `proxy`, ready for the protocol's first byte. The
/// proxy gets `host` as it is, a name or an address, and resolves a name itself. No deadline
/// of its own: the caller's connect timeout covers the handshake.
pub async fn connect(proxy: &Proxy, host: &str, port: u16) -> std::io::Result<TcpStream> {
    let target = match parse_ip_literal(host) {
        Some(ip) => Target::Ip(ip),
        None => Target::Name(host),
    };
    if let (Version::Socks4a, Target::Ip(IpAddr::V6(_))) = (proxy.version, &target) {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!("SOCKS4 proxy {proxy} cannot reach the IPv6 address {target}"),
        ));
    }
    let mut stream = TcpStream::connect((proxy.host.as_str(), proxy.port)).await?;
    match proxy.version {
        Version::Socks5 => socks5(&mut stream, proxy, &target, port).await?,
        Version::Socks4a => socks4a(&mut stream, proxy, &target, port).await?,
    }
    Ok(stream)
}

/// Where the proxy is asked to connect: an address, sent in its canonical form whatever the
/// URL's spelling, or a name.
enum Target<'a> {
    Ip(IpAddr),
    Name(&'a str),
}

impl std::fmt::Display for Target<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Target::Ip(ip) => write!(f, "{ip}"),
            Target::Name(name) => f.write_str(name),
        }
    }
}

async fn socks5(stream: &mut TcpStream, proxy: &Proxy, target: &Target<'_>, port: u16) -> std::io::Result<()> {
    const NO_AUTH: u8 = 0x00;
    const USER_PASS: u8 = 0x02;
    let methods: &[u8] = match proxy.username {
        Some(_) => &[NO_AUTH, USER_PASS],
        None => &[NO_AUTH],
    };
    let mut greeting = vec![5, methods.len() as u8];
    greeting.extend_from_slice(methods);
    stream.write_all(&greeting).await?;
    let mut chosen = [0u8; 2];
    stream.read_exact(&mut chosen).await?;
    if chosen[0] != 5 {
        return Err(refused(proxy, "does not speak SOCKS5"));
    }
    match chosen[1] {
        NO_AUTH => {}
        USER_PASS if proxy.username.is_some() => {
            let user = proxy.username.as_deref().unwrap_or_default().as_bytes();
            let pass = proxy.password.as_deref().unwrap_or_default().as_bytes();
            if user.len() > 255 || pass.len() > 255 {
                return Err(refused(proxy, "takes no username or password over 255 bytes"));
            }
            let mut auth = vec![1, user.len() as u8];
            auth.extend_from_slice(user);
            auth.push(pass.len() as u8);
            auth.extend_from_slice(pass);
            stream.write_all(&auth).await?;
            let mut status = [0u8; 2];
            stream.read_exact(&mut status).await?;
            if status[1] != 0 {
                return Err(refused(proxy, "did not accept the username and password"));
            }
        }
        0xff if proxy.username.is_none() => {
            return Err(refused(proxy, "wants credentials, and the proxy URL has none"))
        }
        _ => return Err(refused(proxy, "accepts none of the authentication methods offered")),
    }

    let mut request = vec![5, 1, 0];
    match target {
        Target::Ip(IpAddr::V4(ip)) => {
            request.push(1);
            request.extend_from_slice(&ip.octets());
        }
        Target::Ip(IpAddr::V6(ip)) => {
            request.push(4);
            request.extend_from_slice(&ip.octets());
        }
        Target::Name(name) => {
            let name = name.as_bytes();
            if name.len() > 255 {
                return Err(Error::new(ErrorKind::InvalidInput, "host name too long for SOCKS5"));
            }
            request.push(3);
            request.push(name.len() as u8);
            request.extend_from_slice(name);
        }
    }
    request.extend_from_slice(&port.to_be_bytes());
    stream.write_all(&request).await?;

    let mut reply = [0u8; 4];
    stream.read_exact(&mut reply).await?;
    if reply[0] != 5 {
        return Err(refused(proxy, "answered in something other than SOCKS5"));
    }
    if reply[1] != 0 {
        let why = match reply[1] {
            1 => "general failure",
            2 => "not allowed by its rules",
            3 => "network unreachable",
            4 => "host unreachable",
            5 => "connection refused",
            6 => "TTL expired",
            7 => "command not supported",
            8 => "address type not supported",
            _ => "unknown error",
        };
        return Err(refused(proxy, &format!("could not connect to {target}:{port}: {why}")));
    }
    // The address the proxy bound, unused.
    let bound = match reply[3] {
        1 => 4,
        4 => 16,
        3 => usize::from(stream.read_u8().await?),
        _ => return Err(refused(proxy, "answered with an unknown address type")),
    };
    let mut rest = vec![0u8; bound + 2];
    stream.read_exact(&mut rest).await?;
    Ok(())
}

async fn socks4a(stream: &mut TcpStream, proxy: &Proxy, target: &Target<'_>, port: u16) -> std::io::Result<()> {
    let mut request = vec![4, 1];
    request.extend_from_slice(&port.to_be_bytes());
    let name = match target {
        Target::Ip(IpAddr::V4(ip)) => {
            request.extend_from_slice(&ip.octets());
            None
        }
        Target::Ip(IpAddr::V6(_)) => return Err(Error::new(ErrorKind::InvalidInput, "IPv6 over SOCKS4")),
        Target::Name(name) => {
            // 0.0.0.x, x not 0: the name follows the user id.
            request.extend_from_slice(&Ipv4Addr::new(0, 0, 0, 1).octets());
            Some(name.as_bytes())
        }
    };
    let user = proxy.username.as_deref().unwrap_or_default().as_bytes();
    if user.contains(&0) || name.is_some_and(|n| n.contains(&0)) {
        return Err(Error::new(ErrorKind::InvalidInput, "NUL in a SOCKS4 field"));
    }
    request.extend_from_slice(user);
    request.push(0);
    if let Some(name) = name {
        request.extend_from_slice(name);
        request.push(0);
    }
    stream.write_all(&request).await?;

    let mut reply = [0u8; 8];
    stream.read_exact(&mut reply).await?;
    if reply[0] != 0 {
        return Err(refused(proxy, "answered in something other than SOCKS4"));
    }
    match reply[1] {
        90 => Ok(()),
        92 | 93 => Err(refused(proxy, "did not accept the user id")),
        _ => Err(refused(proxy, &format!("could not connect to {target}:{port}"))),
    }
}

fn refused(proxy: &Proxy, why: &str) -> Error {
    Error::new(ErrorKind::ConnectionRefused, format!("SOCKS proxy {proxy} {why}"))
}

#[cfg(test)]
pub(crate) mod testing {
    //! A SOCKS proxy on loopback that records what it is asked and then relays.

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    /// What the fake proxy saw of one request.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Asked {
        pub version: u8,
        /// The address type (SOCKS5: 1, 3 or 4) or, for SOCKS4, 3 for a name and 1 otherwise.
        pub kind: u8,
        /// The host as sent: a name, or the address written out.
        pub host: String,
        pub port: u16,
        pub username: Option<String>,
        pub password: Option<String>,
    }

    /// How the fake proxy answers.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Behaviour {
        /// Takes no credentials.
        Open,
        /// Wants `user`/`pass` (SOCKS5) or the user id `user` (SOCKS4).
        Credentials,
        /// Refuses the `CONNECT` (SOCKS5 reply 5, SOCKS4 91).
        Refuse,
        /// Accepts the connection and says nothing.
        Silent,
    }

    /// A proxy for one connection, relaying to `relay_to` (a port on loopback) once it agreed.
    /// Hands back what it was asked, `None` when it never got to a request.
    pub async fn proxy(behaviour: Behaviour, relay_to: Option<u16>) -> (u16, tokio::task::JoinHandle<Option<Asked>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let (mut client, _) = listener.accept().await.unwrap();
            if behaviour == Behaviour::Silent {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                return None;
            }
            let asked = match client.read_u8().await.ok()? {
                5 => socks5(&mut client, behaviour).await?,
                4 => socks4(&mut client, behaviour).await?,
                _ => return None,
            };
            if let Some(relay) = relay_to {
                let mut upstream = TcpStream::connect(("127.0.0.1", relay)).await.unwrap();
                let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
            }
            Some(asked)
        });
        (port, task)
    }

    async fn socks5(client: &mut TcpStream, behaviour: Behaviour) -> Option<Asked> {
        let n = client.read_u8().await.ok()?;
        let mut methods = vec![0u8; n as usize];
        client.read_exact(&mut methods).await.ok()?;
        let (mut username, mut password) = (None, None);
        if behaviour == Behaviour::Credentials {
            if !methods.contains(&2) {
                client.write_all(&[5, 0xff]).await.ok()?;
                return None;
            }
            client.write_all(&[5, 2]).await.ok()?;
            let _ = client.read_u8().await.ok()?;
            let user = read_string(client).await?;
            let pass = read_string(client).await?;
            let ok = user == "user" && pass == "pass";
            client.write_all(&[1, if ok { 0 } else { 1 }]).await.ok()?;
            if !ok {
                return None;
            }
            (username, password) = (Some(user), Some(pass));
        } else {
            client.write_all(&[5, 0]).await.ok()?;
        }
        let mut head = [0u8; 4];
        client.read_exact(&mut head).await.ok()?;
        let host = match head[3] {
            1 => {
                let mut ip = [0u8; 4];
                client.read_exact(&mut ip).await.ok()?;
                std::net::Ipv4Addr::from(ip).to_string()
            }
            4 => {
                let mut ip = [0u8; 16];
                client.read_exact(&mut ip).await.ok()?;
                std::net::Ipv6Addr::from(ip).to_string()
            }
            _ => read_string(client).await?,
        };
        let port = client.read_u16().await.ok()?;
        let rep = if behaviour == Behaviour::Refuse { 5 } else { 0 };
        // Bound to a name, so the client has to read one off.
        client
            .write_all(&[5, rep, 0, 3, 5, b'p', b'r', b'o', b'x', b'y', 0, 0])
            .await
            .ok()?;
        Some(Asked {
            version: 5,
            kind: head[3],
            host,
            port,
            username,
            password,
        })
    }

    async fn socks4(client: &mut TcpStream, behaviour: Behaviour) -> Option<Asked> {
        let _command = client.read_u8().await.ok()?;
        let port = client.read_u16().await.ok()?;
        let mut ip = [0u8; 4];
        client.read_exact(&mut ip).await.ok()?;
        let user = read_until_nul(client).await?;
        let (kind, host) = if ip[..3] == [0, 0, 0] && ip[3] != 0 {
            (3, read_until_nul(client).await?)
        } else {
            (1, std::net::Ipv4Addr::from(ip).to_string())
        };
        let granted = match behaviour {
            Behaviour::Refuse => false,
            Behaviour::Credentials => user == "user",
            _ => true,
        };
        client
            .write_all(&[0, if granted { 90 } else { 91 }, 0, 0, 0, 0, 0, 0])
            .await
            .ok()?;
        granted.then_some(Asked {
            version: 4,
            kind,
            host,
            port,
            username: Some(user).filter(|u| !u.is_empty()),
            password: None,
        })
    }

    async fn read_string(client: &mut TcpStream) -> Option<String> {
        let len = client.read_u8().await.ok()?;
        let mut bytes = vec![0u8; len as usize];
        client.read_exact(&mut bytes).await.ok()?;
        String::from_utf8(bytes).ok()
    }

    async fn read_until_nul(client: &mut TcpStream) -> Option<String> {
        let mut bytes = Vec::new();
        loop {
            match client.read_u8().await.ok()? {
                0 => return String::from_utf8(bytes).ok(),
                b => bytes.push(b),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{proxy, Asked, Behaviour};
    use super::*;
    use std::collections::HashMap;

    fn env(vars: &[(&str, &str)]) -> Proxies {
        let vars: HashMap<String, String> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        Proxies::from_env(|name| vars.get(name).cloned())
    }

    fn socks(port: u16) -> Proxy {
        Proxy::parse(&format!("socks5h://127.0.0.1:{port}")).unwrap().unwrap()
    }

    /// A server on loopback that writes `reply` to whoever connects, after reading a line.
    async fn echo_server(reply: &'static [u8]) -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 64];
            let _ = s.read(&mut buf).await;
            let _ = s.write_all(reply).await;
        });
        port
    }

    #[test]
    fn only_all_proxy_applies_with_the_uppercase_name_first() {
        let p = env(&[("ALL_PROXY", "socks5h://upper:1"), ("all_proxy", "socks5h://lower:1")]);
        assert_eq!(p.for_target("example.org", 70).unwrap().host, "upper");
        // Empty is unset, so the lowercase one counts.
        let p = env(&[("ALL_PROXY", "  "), ("all_proxy", "socks5://lower:1")]);
        assert_eq!(p.for_target("example.org", 70).unwrap().host, "lower");
        // Scoped to http and https URLs in sonar.
        let p = env(&[("HTTP_PROXY", "socks5h://h:1"), ("HTTPS_PROXY", "socks5h://s:1")]);
        assert!(p.for_target("example.org", 70).is_none());
    }

    #[test]
    fn an_http_proxy_leaves_the_connection_direct() {
        assert!(env(&[("ALL_PROXY", "http://proxy:3128")])
            .for_target("a.test", 70)
            .is_none());
        // Without a scheme it is an http proxy.
        assert!(env(&[("ALL_PROXY", "proxy:3128")]).for_target("a.test", 70).is_none());
        assert!(env(&[("ALL_PROXY", "ftp://proxy:21")])
            .for_target("a.test", 70)
            .is_none());
    }

    #[test]
    fn a_proxy_url_is_read_with_its_defaults_and_credentials() {
        let p = Proxy::parse("socks5://us%40er:p%3Ass@[::1]").unwrap().unwrap();
        assert_eq!(p.version, Version::Socks5);
        assert_eq!((p.host.as_str(), p.port), ("::1", 1080));
        assert_eq!(p.username.as_deref(), Some("us@er"));
        assert_eq!(p.password.as_deref(), Some("p:ss"));
        assert_eq!(p.to_string(), "[::1]:1080");
        assert!(!format!("{p:?}").contains("p:ss"));
        let p = Proxy::parse("socks4://tor:9050").unwrap().unwrap();
        assert_eq!(p.version, Version::Socks4a);
        assert_eq!(p.username, None);
    }

    #[test]
    fn no_proxy_is_read_as_sonar_reads_it() {
        let p = |list: &str| env(&[("ALL_PROXY", "socks5h://127.0.0.1:9050"), ("NO_PROXY", list)]);
        let bypassed = |list: &str, host: &str, port: u16| p(list).for_target(host, port).is_none();

        assert!(bypassed("example.org", "example.org", 70));
        assert!(bypassed("example.org", "Gopher.Example.org.", 70));
        assert!(bypassed(".example.org", "example.org", 70));
        assert!(!bypassed("example.org", "notexample.org", 70));
        assert!(bypassed("example.org.", "example.org", 70));
        assert!(bypassed(" a.test , example.org", "example.org", 70));

        assert!(bypassed("*", "anything.test", 70));
        assert!(bypassed("*", "10.1.2.3", 70));
        assert!(bypassed("*", "::1", 70));

        assert!(bypassed("10.0.0.0/8", "10.1.2.3", 70));
        assert!(!bypassed("10.0.0.0/8", "11.1.2.3", 70));
        assert!(bypassed("192.0.2.7", "192.0.2.7", 70));
        assert!(bypassed("[::1]", "::1", 70));
        assert!(bypassed("2001:db8::/32", "2001:db8::5", 70));
        assert!(!bypassed("2001:db8::/32", "10.0.0.1", 70));
        // A name entry does not cover an address.
        assert!(!bypassed("localhost", "127.0.0.1", 70));

        assert!(bypassed("example.org:70", "example.org", 70));
        assert!(!bypassed("example.org:70", "example.org", 1965));
        assert!(bypassed("[::1]:70", "::1", 70));

        assert!(!bypassed("", "example.org", 70));
    }

    #[tokio::test]
    async fn socks5_without_credentials_connects_and_relays() {
        let server = echo_server(b"hello").await;
        let (port, asked) = proxy(Behaviour::Open, Some(server)).await;
        let mut stream = connect(&socks(port), "gopher.invalid", 70).await.unwrap();
        stream.write_all(b"\r\n").await.unwrap();
        let mut body = Vec::new();
        stream.read_to_end(&mut body).await.unwrap();
        assert_eq!(body, b"hello");
        // The relay ends when both sides are closed.
        drop(stream);
        assert_eq!(
            asked.await.unwrap(),
            Some(Asked {
                version: 5,
                kind: 3,
                host: "gopher.invalid".into(),
                port: 70,
                username: None,
                password: None,
            })
        );
    }

    #[tokio::test]
    async fn socks5_presents_the_username_and_password_of_the_proxy_url() {
        let server = echo_server(b"hi").await;
        let (port, asked) = proxy(Behaviour::Credentials, Some(server)).await;
        let p = Proxy::parse(&format!("socks5h://user:pass@127.0.0.1:{port}"))
            .unwrap()
            .unwrap();
        connect(&p, "gopher.invalid", 70).await.unwrap();
        let asked = asked.await.unwrap().unwrap();
        assert_eq!(asked.username.as_deref(), Some("user"));
        assert_eq!(asked.password.as_deref(), Some("pass"));
    }

    #[tokio::test]
    async fn socks5_wrong_or_missing_credentials_end_in_a_clean_error() {
        let (port, _) = proxy(Behaviour::Credentials, None).await;
        let p = Proxy::parse(&format!("socks5h://user:wrong@127.0.0.1:{port}"))
            .unwrap()
            .unwrap();
        let err = connect(&p, "gopher.invalid", 70).await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConnectionRefused);
        assert!(err.to_string().contains("username and password"), "{err}");

        let (port, _) = proxy(Behaviour::Credentials, None).await;
        let err = connect(&socks(port), "gopher.invalid", 70).await.unwrap_err();
        assert!(err.to_string().contains("wants credentials"), "{err}");
    }

    #[tokio::test]
    async fn socks5_sends_an_address_in_its_canonical_form() {
        let (port, asked) = proxy(Behaviour::Open, None).await;
        connect(&socks(port), "0x5db8d70e", 70).await.unwrap();
        let asked = asked.await.unwrap().unwrap();
        assert_eq!((asked.kind, asked.host.as_str()), (1, "93.184.215.14"));

        let (port, asked) = proxy(Behaviour::Open, None).await;
        connect(&socks(port), "2001:db8::1", 70).await.unwrap();
        let asked = asked.await.unwrap().unwrap();
        assert_eq!((asked.kind, asked.host.as_str()), (4, "2001:db8::1"));
    }

    #[tokio::test]
    async fn a_refused_connect_is_a_clean_error_naming_the_proxy() {
        let (port, _) = proxy(Behaviour::Refuse, None).await;
        let err = connect(&socks(port), "gopher.invalid", 70).await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConnectionRefused);
        let message = err.to_string();
        assert!(message.contains(&format!("127.0.0.1:{port}")), "{message}");
        assert!(message.contains("gopher.invalid:70: connection refused"), "{message}");

        let (port, _) = proxy(Behaviour::Refuse, None).await;
        let p = Proxy::parse(&format!("socks4a://127.0.0.1:{port}")).unwrap().unwrap();
        let err = connect(&p, "gopher.invalid", 70).await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConnectionRefused);
    }

    #[tokio::test]
    async fn socks4a_sends_the_name_and_the_user_id() {
        let server = echo_server(b"4a").await;
        let (port, asked) = proxy(Behaviour::Credentials, Some(server)).await;
        let p = Proxy::parse(&format!("socks4://user@127.0.0.1:{port}"))
            .unwrap()
            .unwrap();
        let mut stream = connect(&p, "gopher.invalid", 7070).await.unwrap();
        stream.write_all(b"\r\n").await.unwrap();
        let mut body = Vec::new();
        stream.read_to_end(&mut body).await.unwrap();
        assert_eq!(body, b"4a");
        drop(stream);
        assert_eq!(
            asked.await.unwrap(),
            Some(Asked {
                version: 4,
                kind: 3,
                host: "gopher.invalid".into(),
                port: 7070,
                username: Some("user".into()),
                password: None,
            })
        );
    }

    #[tokio::test]
    async fn socks4a_sends_an_ipv4_address_as_is_and_refuses_ipv6() {
        let (port, asked) = proxy(Behaviour::Open, None).await;
        let p = Proxy::parse(&format!("socks4a://127.0.0.1:{port}")).unwrap().unwrap();
        connect(&p, "93.184.215.14", 70).await.unwrap();
        let asked = asked.await.unwrap().unwrap();
        assert_eq!((asked.kind, asked.host.as_str()), (1, "93.184.215.14"));

        let err = connect(&p, "2001:db8::1", 70).await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }
}
