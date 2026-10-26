//! A gemini server on loopback, for tests here and of the whole engine.
//!
//! The certificates in `testdata` are self-signed EC P-256, for `localhost` and `127.0.0.1`,
//! valid until 2126; the keys are PKCS#8. Both are DER.

use rustls::client::danger::HandshakeSignatureValid;
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls::{DigitallySignedStruct, DistinguishedName, SignatureScheme};
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

/// Asks every client for a certificate, takes any, and checks only that the client holds its
/// key, as a gemini server that knows its users by certificate does.
#[derive(Debug)]
struct AnyClient(WebPkiSupportedAlgorithms);

impl ClientCertVerifier for AnyClient {
    fn client_auth_mandatory(&self) -> bool {
        false
    }

    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.0)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.0)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_schemes()
    }
}

/// How a test server speaks TLS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tls {
    /// Any version, no client certificate asked for.
    Plain,
    /// Any version, a client certificate asked for.
    AskingClients,
    /// TLS 1.2 only, a client certificate asked for.
    AskingClientsOver12,
}

fn acceptor((cert, key): Identity, tls: Tls) -> TlsAcceptor {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let algorithms = provider.signature_verification_algorithms;
    let key = provider
        .key_provider
        .load_private_key(PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key)))
        .unwrap();
    let certified = CertifiedKey::new(vec![CertificateDer::from(cert)], key);
    let builder = rustls::ServerConfig::builder_with_provider(provider);
    let builder = match tls {
        Tls::AskingClientsOver12 => builder.with_protocol_versions(&[&rustls::version::TLS12]),
        _ => builder.with_safe_default_protocol_versions(),
    }
    .unwrap();
    let builder = match tls {
        Tls::Plain => builder.with_no_client_auth(),
        _ => builder.with_client_cert_verifier(Arc::new(AnyClient(algorithms))),
    };
    let config = builder.with_cert_resolver(Arc::new(Fixed(Arc::new(certified))));
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
    let turns: Vec<_> = identities.iter().map(|&identity| (identity, response)).collect();
    serve_each(&turns, clean).await
}

/// [`serve`], with a response of its own for each connection.
pub async fn serve_each(
    turns: &[(Identity, &'static [u8])],
    clean: bool,
) -> (u16, tokio::task::JoinHandle<Vec<Option<Vec<u8>>>>) {
    let (port, task) = serve_tls(turns, Tls::Plain, clean).await;
    let lines = tokio::spawn(async move {
        task.await
            .unwrap()
            .into_iter()
            .map(|turn| turn.map(|(line, _)| line))
            .collect()
    });
    (port, lines)
}

/// [`serve_each`] speaking `tls`, handing back with each request line the client certificate
/// the connection presented, if any.
pub async fn serve_tls(
    turns: &[(Identity, &'static [u8])],
    tls: Tls,
    clean: bool,
) -> (u16, tokio::task::JoinHandle<Vec<Option<(Vec<u8>, Option<Vec<u8>>)>>>) {
    let turns: Vec<(TlsAcceptor, &'static [u8])> = turns
        .iter()
        .map(|&(identity, response)| (acceptor(identity, tls), response))
        .collect();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let mut lines = Vec::new();
        for (acceptor, response) in turns {
            let (tcp, _) = listener.accept().await.unwrap();
            let Ok(mut tls) = acceptor.accept(tcp).await else {
                lines.push(None);
                continue;
            };
            let presented = tls
                .get_ref()
                .1
                .peer_certificates()
                .and_then(|chain| chain.first())
                .map(|der| der.to_vec());
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
            lines.push(Some((line, presented)));
        }
        lines
    });
    (port, task)
}
