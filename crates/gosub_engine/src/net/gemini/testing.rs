//! A gemini server on loopback, for tests here and of the whole engine.
//!
//! The certificates in `testdata` are self-signed EC P-256, for `localhost` and `127.0.0.1`,
//! valid until 2126; the keys are PKCS#8. Both are DER.

use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

pub const A_CERT: &[u8] = include_bytes!("testdata/a.crt.der");
pub const A_KEY: &[u8] = include_bytes!("testdata/a.key.der");
pub const B_CERT: &[u8] = include_bytes!("testdata/b.crt.der");
pub const B_KEY: &[u8] = include_bytes!("testdata/b.key.der");

/// A certificate a server shows, and the key it signs the handshake with.
pub type Identity = (&'static [u8], &'static [u8]);

pub const A: Identity = (A_CERT, A_KEY);
pub const B: Identity = (B_CERT, B_KEY);
/// Certificate A, signed for with B's key: what a server without A's key could show.
pub const FORGED: Identity = (A_CERT, B_KEY);

/// Shows one certificate and signs with whatever key it is handed, matching or not.
#[derive(Debug)]
struct Fixed(Arc<CertifiedKey>);

impl ResolvesServerCert for Fixed {
    fn resolve(&self, _hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(self.0.clone())
    }
}

fn acceptor((cert, key): Identity) -> TlsAcceptor {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let key = provider
        .key_provider
        .load_private_key(PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key)))
        .unwrap();
    let certified = CertifiedKey::new(vec![CertificateDer::from(cert)], key);
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(Fixed(Arc::new(certified))));
    TlsAcceptor::from(Arc::new(config))
}

/// A server answering one connection per entry of `identities`, in turn, as that identity,
/// with `response`, closing with `close_notify` when `clean`. Hands back the request line of
/// each connection, or `None` for one whose handshake failed.
pub async fn serve(
    identities: &[Identity],
    response: &'static [u8],
    clean: bool,
) -> (u16, tokio::task::JoinHandle<Vec<Option<Vec<u8>>>>) {
    let acceptors: Vec<TlsAcceptor> = identities.iter().copied().map(acceptor).collect();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let mut lines = Vec::new();
        for acceptor in acceptors {
            let (tcp, _) = listener.accept().await.unwrap();
            let Ok(mut tls) = acceptor.accept(tcp).await else {
                lines.push(None);
                continue;
            };
            let mut line = Vec::new();
            let mut byte = [0u8; 1];
            while tls.read_exact(&mut byte).await.is_ok() {
                line.push(byte[0]);
                if line.ends_with(b"\r\n") {
                    break;
                }
            }
            let _ = tls.write_all(response).await;
            if clean {
                let _ = tls.shutdown().await;
            }
            lines.push(Some(line));
        }
        lines
    });
    (port, task)
}
