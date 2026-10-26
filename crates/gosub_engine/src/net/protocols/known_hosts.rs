//! Certificates trusted on first use, for a protocol whose servers mostly sign their own.
//!
//! The first connection to a host and port records the certificate it showed ([`CertPin`]);
//! every later one must show the same, or is refused. The store is the zone's: the broker
//! looks a pin up before a request goes out and records the certificate a first visit
//! reports, in this process, since the network process keeps nothing. An embedder that
//! wants pins to outlive the process passes a [`KnownHosts`] to the zone builder, such as
//! [`SqliteKnownHosts`] (feature `sqlite_known_hosts`); the default keeps them in memory.
//!
//! A server showing another certificate than the pinned one is refused with a TLS error
//! carrying the new certificate's fingerprint. An embedder that lets the user trust it
//! stores exactly that one ([`CertPin::from_digest`], [`KnownHosts::remember`]) under the
//! page's [`key_for`], rather than forgetting the pin and pinning whatever comes next.

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

    /// The SHA-256 itself, as a TLS error's fingerprint carries it.
    pub fn digest(&self) -> [u8; 32] {
        let mut digest = [0u8; 32];
        for (byte, pair) in digest.iter_mut().zip(self.0.as_bytes().chunks(2)) {
            // Two lowercase hex digits each: `of` and `parse` make no other pin.
            let hex = |c: u8| if c.is_ascii_digit() { c - b'0' } else { c - b'a' + 10 };
            *byte = hex(pair[0]) << 4 | hex(pair[1]);
        }
        digest
    }

    /// The pin a fingerprint stands for.
    pub fn from_digest(digest: &[u8; 32]) -> Self {
        Self(digest.iter().map(|b| format!("{b:02x}")).collect())
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
    /// Record `pin` for `host`:`port`, replacing any there: also how an embedder trusts the
    /// new certificate a refused visit named.
    fn remember(&self, host: &str, port: u16, pin: &CertPin);
    /// Drop the pin for `host`:`port`, so the next connection is a first visit again, as
    /// when the user clears what the profile remembers.
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

/// The host and port a pin for `url` is kept under, when a protocol in this build pins
/// certificates for its scheme: what an embedder passes to [`KnownHosts`] for a page.
pub fn key_for(url: &Url) -> Option<(String, u16)> {
    let protocol = crate::net::protocols::lookup(url.scheme()).filter(|p| p.pins_certificates())?;
    key(url, protocol.default_port())
}

#[cfg(feature = "sqlite_known_hosts")]
pub use sqlite::SqliteKnownHosts;

#[cfg(feature = "sqlite_known_hosts")]
mod sqlite {
    use super::{CertPin, KnownHosts};
    use crate::EngineError;
    use cow_utils::CowUtils;
    use r2d2::Pool;
    use r2d2_sqlite::rusqlite::{self, params, Connection, OptionalExtension};
    use r2d2_sqlite::SqliteConnectionManager;
    use std::path::PathBuf;

    /// Pins in a SQLite database, so they outlive the process. Access goes through an `r2d2`
    /// pool, like the cookie store. A database error is logged and the store behaves as
    /// empty for that call: a pin that cannot be read makes the visit a first one, which
    /// the user is no worse off for than with the default store.
    pub struct SqliteKnownHosts {
        pool: Pool<SqliteConnectionManager>,
    }

    impl std::fmt::Debug for SqliteKnownHosts {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("SqliteKnownHosts").finish_non_exhaustive()
        }
    }

    impl SqliteKnownHosts {
        /// Opens (or creates) the database at `path`.
        pub fn new(path: PathBuf) -> Result<Self, EngineError> {
            let manager = SqliteConnectionManager::file(path);
            let pool = Pool::new(manager).map_err(|e| EngineError::Internal(anyhow::anyhow!(e)))?;
            pool.get()
                .map_err(|e| EngineError::Internal(anyhow::anyhow!(e)))?
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS known_hosts (
                         host TEXT    NOT NULL,
                         port INTEGER NOT NULL,
                         pin  TEXT    NOT NULL,
                         PRIMARY KEY (host, port)
                     );",
                )
                .map_err(|e| EngineError::Internal(anyhow::anyhow!(e)))?;
            Ok(Self { pool })
        }

        /// `f` on a pooled connection; `None`, logged, when either fails.
        fn run<T>(&self, what: &str, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Option<T> {
            let conn = match self.pool.get() {
                Ok(conn) => conn,
                Err(e) => {
                    log::error!("known hosts: no database connection to {what}: {e}");
                    return None;
                }
            };
            f(&conn)
                .inspect_err(|e| log::error!("known hosts: could not {what}: {e}"))
                .ok()
        }
    }

    impl KnownHosts for SqliteKnownHosts {
        fn pin(&self, host: &str, port: u16) -> Option<CertPin> {
            let host = host.cow_to_ascii_lowercase();
            let text = self.run("read a pin", |conn| {
                conn.query_row(
                    "SELECT pin FROM known_hosts WHERE host = ?1 AND port = ?2",
                    params![&*host, port],
                    |row| row.get::<_, String>(0),
                )
                .optional()
            })??;
            // A row that is no pin is treated as none, never as a pin of nothing.
            let pin = CertPin::parse(&text);
            if pin.is_none() {
                log::error!("known hosts: the pin stored for {host}:{port} is malformed");
            }
            pin
        }

        fn remember(&self, host: &str, port: u16, pin: &CertPin) {
            let host = host.cow_to_ascii_lowercase();
            self.run("store a pin", |conn| {
                conn.execute(
                    "INSERT INTO known_hosts (host, port, pin) VALUES (?1, ?2, ?3)
                     ON CONFLICT (host, port) DO UPDATE SET pin = excluded.pin",
                    params![&*host, port, pin.as_str()],
                )
            });
        }

        fn forget(&self, host: &str, port: u16) {
            let host = host.cow_to_ascii_lowercase();
            self.run("forget a pin", |conn| {
                conn.execute(
                    "DELETE FROM known_hosts WHERE host = ?1 AND port = ?2",
                    params![&*host, port],
                )
            });
        }
    }
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
    fn a_pin_and_its_digest_are_the_same_certificate() {
        let pin = CertPin::of(b"cert");
        let digest: [u8; 32] = Sha256::digest(b"cert").into();
        assert_eq!(pin.digest(), digest);
        assert_eq!(CertPin::from_digest(&digest), pin);
    }

    #[cfg(feature = "gemini")]
    #[test]
    fn a_page_s_key_uses_its_protocol_s_port() {
        let url = |s: &str| Url::parse(s).unwrap();
        assert_eq!(key_for(&url("gemini://Host.Test/x")), Some(("host.test".into(), 1965)));
        assert_eq!(
            key_for(&url("gemini://host.test:7000/")),
            Some(("host.test".into(), 7000))
        );
        // Only a protocol that pins has keys.
        assert_eq!(key_for(&url("https://host.test/")), None);
        #[cfg(feature = "gopher")]
        assert_eq!(key_for(&url("gopher://host.test/")), None);
    }

    #[cfg(feature = "sqlite_known_hosts")]
    #[test]
    fn pins_in_sqlite_outlive_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("known_hosts.db");
        let (one, two) = (CertPin::of(b"one"), CertPin::of(b"two"));
        {
            let hosts = SqliteKnownHosts::new(path.clone()).unwrap();
            assert_eq!(hosts.pin("host.test", 1965), None);
            hosts.remember("Host.Test", 1965, &one);
            hosts.remember("other.test", 1965, &one);
            // Trusting a new certificate replaces the pin.
            hosts.remember("host.test", 1965, &two);
            hosts.forget("other.test", 1965);
        }
        let hosts = SqliteKnownHosts::new(path).unwrap();
        assert_eq!(hosts.pin("host.test", 1965), Some(two));
        assert_eq!(hosts.pin("host.test", 1966), None);
        assert_eq!(hosts.pin("other.test", 1965), None);
    }

    #[cfg(feature = "sqlite_known_hosts")]
    #[test]
    fn a_malformed_stored_pin_is_no_pin() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("known_hosts.db");
        let hosts = SqliteKnownHosts::new(path.clone()).unwrap();
        r2d2_sqlite::rusqlite::Connection::open(&path)
            .unwrap()
            .execute(
                "INSERT INTO known_hosts (host, port, pin) VALUES ('host.test', 1965, 'nope')",
                [],
            )
            .unwrap();
        assert_eq!(hosts.pin("host.test", 1965), None);
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
