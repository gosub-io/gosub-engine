//! Client certificates a zone presents to servers that ask for one.
//!
//! A gemini server answers `60` when a page needs a client certificate, and `61` or `62` when
//! the one it got will not do. The engine never makes one up by itself: the specification
//! forbids a client to generate a certificate and repeat the request without the user taking
//! part. The navigation fails instead (`LoadError::ClientCertificate`), and an embedder that
//! asks the user creates one ([`ClientIdentity::generate`]) and adds it to the zone's store
//! ([`ClientIdentities::add`]).
//!
//! An identity is for the host, port and path of the URL that asked, and every path below it
//! ([`IdentityScope`]), as the specification scopes a certificate made in answer to a `60`. A
//! request presents the identity whose scope covers its URL most narrowly, and on a redirect
//! only while the redirect stays inside that scope: never to another server, and never above
//! the path it was made for.
//!
//! The store is the zone's and lives in this process, like the pins (`known_hosts`). With the
//! network process on, the identity a request presents goes with it, private key and all:
//! that process makes the TLS connection, so it has to sign with the key. It is sent only the
//! one identity the request presents.

use parking_lot::Mutex;
use std::sync::Arc;
use url::Url;
use zeroize::Zeroizing;

/// Where an identity is presented: a host and port, and a path with every path below it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IdentityScope {
    /// The host without IPv6 brackets, in lowercase.
    pub host: String,
    pub port: u16,
    /// The path the identity was made for, `/` for a whole server.
    pub path: String,
}

impl IdentityScope {
    /// The scope of an identity made for `url`, over an engine protocol in this build that
    /// presents client certificates: its host, port and path, without the query.
    pub fn for_url(url: &Url) -> Option<Self> {
        let protocol = crate::net::protocols::lookup(url.scheme()).filter(|p| p.pins_certificates())?;
        Self::of(url, protocol.default_port())
    }

    fn of(url: &Url, default_port: u16) -> Option<Self> {
        let (host, port) = crate::net::protocols::known_hosts::key(url, default_port)?;
        Some(Self {
            host,
            port,
            path: path_of(url).to_string(),
        })
    }

    /// Whether a request to `url` over a protocol whose port is `default_port` is inside the
    /// scope: the same host and port, and the scope's path or one below it. `/app` covers
    /// `/app` and `/app/x`, never `/apple`.
    pub fn covers(&self, url: &Url, default_port: u16) -> bool {
        let Some(other) = Self::of(url, default_port) else {
            return false;
        };
        if other.host != self.host || other.port != self.port {
            return false;
        }
        match other.path.strip_prefix(self.path.as_str()) {
            Some(rest) => rest.is_empty() || self.path.ends_with('/') || rest.starts_with('/'),
            None => false,
        }
    }
}

/// A URL's path, `/` for an empty one: the specification makes the two the same page.
fn path_of(url: &Url) -> &str {
    match url.path() {
        "" => "/",
        path => path,
    }
}

/// A certificate and its private key, presented within a scope.
#[derive(Clone, PartialEq, Eq)]
pub struct ClientIdentity {
    pub scope: IdentityScope,
    /// What the user called it; also the certificate's common name.
    pub name: String,
    /// The certificate, in DER.
    pub certificate: Vec<u8>,
    /// Its private key, PKCS#8 in DER. Wiped from memory when dropped.
    pub key: Zeroizing<Vec<u8>>,
}

impl std::fmt::Debug for ClientIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientIdentity")
            .field("scope", &self.scope)
            .field("name", &self.name)
            .field(
                "certificate",
                &crate::net::protocols::known_hosts::CertPin::of(&self.certificate),
            )
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "gemini")]
impl ClientIdentity {
    /// How long a generated certificate is valid. Servers mostly know a client by its
    /// certificate, so one that runs out is an account lost.
    pub const VALIDITY_DAYS: i64 = 20 * 365;

    /// A new self-signed certificate named `name`, with a fresh ECDSA P-256 key, for
    /// `scope`. Only ever on the user's say-so: see the module documentation.
    pub fn generate(scope: IdentityScope, name: &str) -> Result<Self, crate::EngineError> {
        let failed = |e: rcgen::Error| crate::EngineError::Internal(anyhow::anyhow!("generating a certificate: {e}"));
        let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).map_err(failed)?;
        let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).map_err(failed)?;
        params.distinguished_name = rcgen::DistinguishedName::new();
        params.distinguished_name.push(rcgen::DnType::CommonName, name);
        let now = time::OffsetDateTime::now_utc();
        params.not_before = now - time::Duration::days(1);
        params.not_after = now + time::Duration::days(Self::VALIDITY_DAYS);
        let certificate = params.self_signed(&key).map_err(failed)?;
        Ok(Self {
            scope,
            name: name.to_string(),
            certificate: certificate.der().to_vec(),
            key: Zeroizing::new(key.serialize_der()),
        })
    }
}

/// A server's answer that it wants a client certificate: `60` when none came, `61` or `62`
/// when the one presented will not do. What a navigation fails with, so the embedder can ask
/// the user; `url` is where the answer came from, the scope an identity made for it has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityAsked {
    pub url: Url,
    pub status: u8,
    /// The server's message, which the specification has a client show.
    pub message: String,
    /// Whether the request presented an identity.
    pub presented: bool,
}

impl std::fmt::Display for IdentityAsked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let asked = match (self.status, self.presented) {
            (61, _) => "is not authorised by its client certificate",
            (62, _) => "did not accept its client certificate as valid",
            (_, true) => "wants another client certificate",
            (_, false) => "asks for a client certificate",
        };
        write!(f, "{} {asked} ({})", self.url, self.status)?;
        if !self.message.is_empty() {
            write!(f, ": {}", self.message)?;
        }
        Ok(())
    }
}

impl std::error::Error for IdentityAsked {}

impl IdentityAsked {
    /// The failure a fetch ends in for it.
    pub fn into_net_error(self) -> crate::net::types::NetError {
        crate::net::types::NetError::Other(Arc::new(anyhow::Error::new(self)))
    }

    /// The answer `error` carries, if it is one.
    pub fn of(error: &crate::net::types::NetError) -> Option<&Self> {
        match error {
            crate::net::types::NetError::Other(e) => e.downcast_ref(),
            _ => None,
        }
    }
}

/// Where a zone keeps its client identities, one per scope.
pub trait ClientIdentities: Send + Sync + std::fmt::Debug {
    /// Every identity, in no particular order.
    fn all(&self) -> Vec<ClientIdentity>;
    /// Keep `identity`, replacing any with the same scope.
    fn add(&self, identity: ClientIdentity);
    /// Drop the identity for `scope`, if there is one.
    fn remove(&self, scope: &IdentityScope);

    /// The identity a request to `url` over a protocol whose port is `default_port`
    /// presents: of those whose scope covers it, the one with the longest path.
    fn for_url(&self, url: &Url, default_port: u16) -> Option<ClientIdentity> {
        self.all()
            .into_iter()
            .filter(|identity| identity.scope.covers(url, default_port))
            .max_by_key(|identity| identity.scope.path.len())
    }
}

pub type ClientIdentitiesHandle = Arc<dyn ClientIdentities>;

/// Identities in memory, for as long as the zone lives.
#[derive(Debug, Default)]
pub struct InMemoryClientIdentities {
    identities: Mutex<Vec<ClientIdentity>>,
}

impl InMemoryClientIdentities {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn handle() -> ClientIdentitiesHandle {
        Arc::new(Self::new())
    }
}

impl ClientIdentities for InMemoryClientIdentities {
    fn all(&self) -> Vec<ClientIdentity> {
        self.identities.lock().clone()
    }

    fn add(&self, identity: ClientIdentity) {
        let mut identities = self.identities.lock();
        identities.retain(|kept| kept.scope != identity.scope);
        identities.push(identity);
    }

    fn remove(&self, scope: &IdentityScope) {
        self.identities.lock().retain(|kept| kept.scope != *scope);
    }
}

#[cfg(feature = "sqlite_client_identities")]
pub use sqlite::SqliteClientIdentities;

#[cfg(feature = "sqlite_client_identities")]
mod sqlite {
    use super::{ClientIdentities, ClientIdentity, IdentityScope};
    use crate::EngineError;
    use r2d2::Pool;
    use r2d2_sqlite::rusqlite::{self, params, Connection};
    use r2d2_sqlite::SqliteConnectionManager;
    use std::path::PathBuf;
    use zeroize::Zeroizing;

    /// Identities in a SQLite database, so they outlive the process. The private keys are
    /// stored as they are: an embedder that keeps the profile on disk encrypts the database,
    /// as for the cookie store. A database error is logged and the store behaves as empty for
    /// that call, which leaves the user without the identity rather than with another one.
    pub struct SqliteClientIdentities {
        pool: Pool<SqliteConnectionManager>,
    }

    impl std::fmt::Debug for SqliteClientIdentities {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("SqliteClientIdentities").finish_non_exhaustive()
        }
    }

    impl SqliteClientIdentities {
        /// Opens (or creates) the database at `path`.
        pub fn new(path: PathBuf) -> Result<Self, EngineError> {
            let manager = SqliteConnectionManager::file(path);
            let pool = Pool::new(manager).map_err(|e| EngineError::Internal(anyhow::anyhow!(e)))?;
            pool.get()
                .map_err(|e| EngineError::Internal(anyhow::anyhow!(e)))?
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS client_identities (
                         host        TEXT    NOT NULL,
                         port        INTEGER NOT NULL,
                         path        TEXT    NOT NULL,
                         name        TEXT    NOT NULL,
                         certificate BLOB    NOT NULL,
                         key         BLOB    NOT NULL,
                         PRIMARY KEY (host, port, path)
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
                    log::error!("client identities: no database connection to {what}: {e}");
                    return None;
                }
            };
            f(&conn)
                .inspect_err(|e| log::error!("client identities: could not {what}: {e}"))
                .ok()
        }
    }

    impl ClientIdentities for SqliteClientIdentities {
        fn all(&self) -> Vec<ClientIdentity> {
            self.run("read the identities", |conn| {
                let mut statement =
                    conn.prepare("SELECT host, port, path, name, certificate, key FROM client_identities")?;
                let rows = statement.query_map([], |row| {
                    Ok(ClientIdentity {
                        scope: IdentityScope {
                            host: row.get(0)?,
                            port: row.get(1)?,
                            path: row.get(2)?,
                        },
                        name: row.get(3)?,
                        certificate: row.get(4)?,
                        key: Zeroizing::new(row.get(5)?),
                    })
                })?;
                rows.collect()
            })
            .unwrap_or_default()
        }

        fn add(&self, identity: ClientIdentity) {
            let ClientIdentity {
                scope,
                name,
                certificate,
                key,
            } = identity;
            self.run("store an identity", |conn| {
                conn.execute(
                    "INSERT INTO client_identities (host, port, path, name, certificate, key)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT (host, port, path) DO UPDATE SET
                         name = excluded.name, certificate = excluded.certificate, key = excluded.key",
                    params![scope.host, scope.port, scope.path, name, certificate, key.as_slice()],
                )
            });
        }

        fn remove(&self, scope: &IdentityScope) {
            self.run("remove an identity", |conn| {
                conn.execute(
                    "DELETE FROM client_identities WHERE host = ?1 AND port = ?2 AND path = ?3",
                    params![scope.host, scope.port, scope.path],
                )
            });
        }
    }
}

/// The certificates a zone keeps for the engine's own protocols: those servers showed, which
/// it trusts on first use, and its own, which it presents.
#[derive(Debug, Clone)]
pub struct ZoneCertificates {
    pub known_hosts: crate::net::protocols::known_hosts::KnownHostsHandle,
    pub identities: ClientIdentitiesHandle,
}

impl Default for ZoneCertificates {
    /// Both in memory, for the zone's life.
    fn default() -> Self {
        Self {
            known_hosts: crate::net::protocols::known_hosts::InMemoryKnownHosts::handle(),
            identities: InMemoryClientIdentities::handle(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    fn scope(path: &str) -> IdentityScope {
        IdentityScope {
            host: "h.test".into(),
            port: 1965,
            path: path.into(),
        }
    }

    fn identity(path: &str) -> ClientIdentity {
        ClientIdentity {
            scope: scope(path),
            name: path.into(),
            certificate: path.as_bytes().to_vec(),
            key: Zeroizing::new(b"key".to_vec()),
        }
    }

    #[test]
    fn a_scope_covers_its_path_and_the_paths_below_it() {
        let covers = |path: &str, at: &str| scope(path).covers(&url(at), 1965);
        assert!(covers("/private/", "gemini://h.test/private/"));
        assert!(covers("/private/", "gemini://h.test/private/r2/r3?q"));
        assert!(covers("/app", "gemini://h.test/app"));
        assert!(covers("/app", "gemini://h.test/app/x"));
        assert!(!covers("/app", "gemini://h.test/apple"));
        assert!(!covers("/private/", "gemini://h.test/private"));
        assert!(!covers("/private/", "gemini://h.test/other/"));
        assert!(covers("/", "gemini://H.Test"));
        // Never another server.
        assert!(!covers("/", "gemini://h.test:1966/"));
        assert!(!covers("/", "gemini://other.test/"));
    }

    #[test]
    fn a_scope_is_made_from_the_url_that_asked() {
        #[cfg(feature = "gemini")]
        assert_eq!(
            IdentityScope::for_url(&url("gemini://H.Test/private/x?q#f")),
            Some(scope("/private/x"))
        );
        assert_eq!(IdentityScope::for_url(&url("https://h.test/")), None);
    }

    #[test]
    fn the_narrowest_identity_that_covers_a_url_is_presented() {
        let store = InMemoryClientIdentities::new();
        store.add(identity("/"));
        store.add(identity("/app/"));
        store.add(identity("/app/admin"));
        let presented = |at: &str| store.for_url(&url(at), 1965).map(|i| i.scope.path);
        assert_eq!(presented("gemini://h.test/app/admin/x").as_deref(), Some("/app/admin"));
        assert_eq!(presented("gemini://h.test/app/x").as_deref(), Some("/app/"));
        assert_eq!(presented("gemini://h.test/other").as_deref(), Some("/"));
        assert_eq!(presented("gemini://other.test/app/"), None);
    }

    #[test]
    fn an_identity_replaces_the_one_with_its_scope() {
        let store = InMemoryClientIdentities::new();
        store.add(identity("/a"));
        let mut again = identity("/a");
        again.name = "again".into();
        store.add(again);
        assert_eq!(store.all().len(), 1);
        assert_eq!(store.all()[0].name, "again");
        store.remove(&scope("/a"));
        assert!(store.all().is_empty());
    }

    #[test]
    fn an_identity_s_key_is_not_in_its_debug_output() {
        let shown = format!("{:?}", identity("/a"));
        assert!(!shown.contains("key"), "{shown}");
    }

    #[cfg(feature = "gemini")]
    #[test]
    fn a_generated_identity_is_a_self_signed_certificate_for_its_key() {
        let made = ClientIdentity::generate(scope("/"), "Visitor").unwrap();
        assert_eq!(made.name, "Visitor");
        // The key parses as the provider's, and its public half is the certificate's.
        let provider = rustls::crypto::aws_lc_rs::default_provider();
        let key = provider
            .key_provider
            .load_private_key(rustls::pki_types::PrivateKeyDer::Pkcs8(made.key.to_vec().into()))
            .unwrap();
        let certified = rustls::sign::CertifiedKey::new(vec![made.certificate.clone().into()], key);
        assert!(certified.keys_match().is_ok());
        let other = ClientIdentity::generate(scope("/"), "Visitor").unwrap();
        assert_ne!(other.certificate, made.certificate);
    }

    #[cfg(feature = "sqlite_client_identities")]
    #[test]
    fn identities_in_sqlite_outlive_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identities.db");
        {
            let store = SqliteClientIdentities::new(path.clone()).unwrap();
            store.add(identity("/a"));
            store.add(identity("/b"));
            store.remove(&scope("/b"));
            let mut again = identity("/a");
            again.name = "again".into();
            store.add(again);
        }
        let store = SqliteClientIdentities::new(path).unwrap();
        let all = store.all();
        assert_eq!(all.len(), 1);
        assert_eq!((all[0].name.as_str(), all[0].key.as_slice()), ("again", &b"key"[..]));
        assert_eq!(
            store.for_url(&url("gemini://h.test/a/x"), 1965).map(|i| i.scope),
            Some(scope("/a"))
        );
    }
}
