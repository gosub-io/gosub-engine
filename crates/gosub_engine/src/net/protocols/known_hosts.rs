//! Certificates trusted on first use, for a protocol whose servers mostly sign their own.
//!
//! The first connection to a host and port records the certificate it showed ([`CertPin`]);
//! every later one must show the same, or is refused. The store is the zone's: the broker
//! looks a pin up before a request goes out and records the certificate a first visit
//! reports, in this process, since the network process keeps nothing. An embedder that
//! wants pins to outlive the process passes its own [`KnownHosts`] to the zone builder; the
//! default keeps them in memory.

use cow_utils::CowUtils;
use parking_lot::Mutex;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use url::Url;

/// A server certificate, as SHA-256 of its DER encoding in lowercase hex.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CertPin(String);

impl CertPin {
    /// The pin of a certificate in DER.
    pub fn of(der: &[u8]) -> Self {
        Self(Sha256::digest(der).iter().map(|b| format!("{b:02x}")).collect())
    }

    /// A pin from its text form: 64 lowercase hex digits, and nothing else.
    pub fn parse(s: &str) -> Option<Self> {
        (s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))).then(|| Self(s.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for CertPin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Where a zone keeps the certificates it has seen, by host and port.
pub trait KnownHosts: Send + Sync + std::fmt::Debug {
    /// The certificate first seen at `host`:`port`, if any.
    fn pin(&self, host: &str, port: u16) -> Option<CertPin>;
    /// Record `pin` for `host`:`port`.
    fn remember(&self, host: &str, port: u16, pin: &CertPin);
    /// Drop the pin for `host`:`port`, so the next connection is a first visit again: what
    /// an embedder offers when the user accepts a server's new certificate.
    fn forget(&self, host: &str, port: u16);
}

pub type KnownHostsHandle = Arc<dyn KnownHosts>;

/// Pins in memory, for as long as the zone lives.
#[derive(Debug, Default)]
pub struct InMemoryKnownHosts {
    pins: Mutex<HashMap<(String, u16), CertPin>>,
}

impl InMemoryKnownHosts {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn handle() -> KnownHostsHandle {
        Arc::new(Self::new())
    }
}

impl KnownHosts for InMemoryKnownHosts {
    fn pin(&self, host: &str, port: u16) -> Option<CertPin> {
        self.pins
            .lock()
            .get(&(host.cow_to_ascii_lowercase().into_owned(), port))
            .cloned()
    }

    fn remember(&self, host: &str, port: u16, pin: &CertPin) {
        self.pins
            .lock()
            .insert((host.cow_to_ascii_lowercase().into_owned(), port), pin.clone());
    }

    fn forget(&self, host: &str, port: u16) {
        self.pins
            .lock()
            .remove(&(host.cow_to_ascii_lowercase().into_owned(), port));
    }
}

/// One request's place in a zone's pins: the pin it goes out with, and where the
/// certificate a first visit reports is recorded.
#[derive(Debug, Clone)]
pub struct PinSlot {
    store: KnownHostsHandle,
    host: String,
    port: u16,
    /// The certificate the server must show, if the zone has seen one there.
    pub pin: Option<CertPin>,
}

impl PinSlot {
    /// The slot for a request to `url` over a protocol whose port is `default_port`.
    pub fn for_url(store: &KnownHostsHandle, url: &Url, default_port: u16) -> Option<Self> {
        let (host, port) = key(url, default_port)?;
        Some(Self {
            pin: store.pin(&host, port),
            store: Arc::clone(store),
            host,
            port,
        })
    }

    /// Record the certificate a response showed, when nothing was pinned here yet. A pin
    /// that was there stays: the protocol refused a different certificate already.
    pub fn record(&self, seen: Option<&CertPin>) {
        if let Some(seen) = seen {
            if self.store.pin(&self.host, self.port).is_none() {
                self.store.remember(&self.host, self.port, seen);
            }
        }
    }
}

/// The host and port a pin for `url` is kept under: the host without IPv6 brackets, in
/// lowercase, and the URL's port or `default_port`. The same key on both sides of the
/// network-process boundary.
pub fn key(url: &Url, default_port: u16) -> Option<(String, u16)> {
    let host = url.host_str()?;
    let host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .cow_to_ascii_lowercase()
        .into_owned();
    if host.is_empty() {
        return None;
    }
    Some((host, url.port().unwrap_or(default_port)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pin_is_the_certificate_s_sha256_in_hex() {
        // SHA-256 of the empty input.
        assert_eq!(
            CertPin::of(b"").as_str(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn only_a_pin_s_own_form_parses() {
        let pin = CertPin::of(b"cert");
        assert_eq!(CertPin::parse(pin.as_str()), Some(pin.clone()));
        assert_eq!(CertPin::parse(&pin.as_str().cow_to_uppercase()), None);
        assert_eq!(CertPin::parse(&pin.as_str()[1..]), None);
        assert_eq!(CertPin::parse(&format!("{}0", pin.as_str())), None);
        assert_eq!(CertPin::parse(&"g".repeat(64)), None);
    }

    #[test]
    fn pins_are_kept_by_host_and_port() {
        let hosts = InMemoryKnownHosts::new();
        let pin = CertPin::of(b"cert");
        hosts.remember("Gemini.Test", 1965, &pin);
        assert_eq!(hosts.pin("gemini.test", 1965), Some(pin));
        assert_eq!(hosts.pin("gemini.test", 1966), None);
        hosts.forget("gemini.test", 1965);
        assert_eq!(hosts.pin("gemini.test", 1965), None);
    }

    #[test]
    fn a_first_visit_is_recorded_and_a_pin_kept() {
        let store = InMemoryKnownHosts::handle();
        let url = Url::parse("gemini://host.test/").unwrap();
        let first = PinSlot::for_url(&store, &url, 1965).unwrap();
        assert_eq!(first.pin, None);
        first.record(Some(&CertPin::of(b"one")));
        let second = PinSlot::for_url(&store, &url, 1965).unwrap();
        assert_eq!(second.pin, Some(CertPin::of(b"one")));
        second.record(Some(&CertPin::of(b"two")));
        assert_eq!(store.pin("host.test", 1965), Some(CertPin::of(b"one")));
    }

    #[test]
    fn the_key_is_the_bare_host_and_the_port() {
        let url = |s: &str| Url::parse(s).unwrap();
        assert_eq!(
            key(&url("gemini://Host.Test/x"), 1965),
            Some(("host.test".into(), 1965))
        );
        assert_eq!(key(&url("gemini://[::1]:7000/"), 1965), Some(("::1".into(), 7000)));
        assert_eq!(key(&url("about:blank"), 1965), None);
    }
}
