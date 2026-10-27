//! Gemini's TLS: a handshake that trusts a server's certificate on first use.
//!
//! Gemini servers mostly sign their own certificates, so there is no authority to check one
//! against. A first visit to a host and port accepts whatever certificate the server shows and
//! reports it, for the broker to pin; a later visit must see the same certificate, or the
//! handshake fails before the request is sent. The handshake's signature is still checked
//! against the certificate's key, so a server can show a certificate only by holding its key.
//!
//! Nothing else about the certificate is judged: not its name and not its dates. The pin is
//! what makes a visit reach the same server as the first one did.
//!
//! A request that presents a client certificate speaks TLS 1.3 only. Over TLS 1.2 the client
//! certificate goes out unencrypted, which the specification has a client warn about; a
//! handshake cannot stop to ask, so a server that speaks only 1.2 is refused instead.

use crate::net::protocols::identities::ClientIdentity;
use crate::net::protocols::known_hosts::CertPin;
use crate::net::protocols::socket::{self, invalid, CONNECT_TIMEOUT};
use crate::net::types::NetError;
use gosub_sonar::net::tls::{TlsError, TlsErrorKind};
use parking_lot::Mutex;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{CertificateError, ClientConfig, DigitallySignedStruct, SignatureScheme};
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;
use tokio_rustls::TlsConnector;

/// A TLS session over `tcp` with `host`, whose certificate must be `pin` when there is one,
/// presenting `identity` when the server asks for a client certificate. Returns the stream
/// and the certificate the server showed.
pub async fn handshake(
    host: &str,
    port: u16,
    tcp: TcpStream,
    pin: Option<&CertPin>,
    identity: Option<&ClientIdentity>,
) -> Result<(TlsStream<TcpStream>, CertPin), NetError> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let verifier = Arc::new(TofuVerifier {
        pin: pin.cloned(),
        algorithms: provider.signature_verification_algorithms,
        seen: Mutex::new(None),
    });
    let builder = ClientConfig::builder_with_provider(provider);
    let builder = match identity {
        Some(_) => builder.with_protocol_versions(&[&rustls::version::TLS13]),
        None => builder.with_safe_default_protocol_versions(),
    }
    .map_err(|e| invalid(format!("TLS setup: {e}")))?
    .dangerous()
    .with_custom_certificate_verifier(verifier.clone());
    let config = match identity {
        Some(identity) => builder
            .with_client_auth_cert(
                vec![CertificateDer::from(identity.certificate.clone())],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(identity.key.to_vec())),
            )
            .map_err(|e| {
                invalid(format!(
                    "the client certificate {:?} cannot be used: {e}",
                    identity.name
                ))
            })?,
        None => builder.with_no_client_auth(),
    };
    // An address is sent no SNI; a name is.
    let server_name =
        ServerName::try_from(host.to_string()).map_err(|_| invalid(format!("{host} is not a TLS server name")))?;
    let connect = TlsConnector::from(Arc::new(config)).connect(server_name, tcp);
    let stream = match tokio::time::timeout(CONNECT_TIMEOUT, connect).await {
        Err(_) => return Err(NetError::Timeout(format!("TLS handshake with {host}"))),
        Ok(Err(e)) => {
            let failed = failure(host, port, pin, verifier.seen.lock().take(), e);
            return Err(match (failed, identity) {
                (NetError::Tls(mut tls), Some(_)) if tls.kind == TlsErrorKind::Handshake => {
                    tls.message = format!(
                        "{} (a client certificate goes out over TLS 1.3 only, and {host}:{port} may \
speak only 1.2, over which it would not be encrypted)",
                        tls.message
                    );
                    NetError::Tls(tls)
                }
                (failed, _) => failed,
            });
        }
        Ok(Ok(stream)) => stream,
    };
    let certificate = stream
        .get_ref()
        .1
        .peer_certificates()
        .and_then(|chain| chain.first())
        .map(|der| CertPin::of(der.as_ref()))
        .ok_or_else(|| invalid(format!("{host} showed no certificate")))?;
    // The pin is checked where the handshake is signed; a handshake that was not, which none
    // rustls speaks is, is still held to it.
    if pin.is_some_and(|pin| *pin != certificate) {
        return Err(NetError::Tls(TlsError {
            kind: TlsErrorKind::InvalidCertificate,
            host: host.to_string(),
            message: format!("{host}:{port} showed another certificate than the pinned one, unsigned"),
            certificate: None,
            fingerprint: None,
        }));
    }
    Ok((stream, certificate))
}

/// The error a failed handshake ends in. A certificate other than the pinned one is said so,
/// with both, and carries the new one's fingerprint: that is what the user has to decide
/// about, and what an embedder pins when they trust it (`known_hosts`). Any other failure
/// carries none, so nothing else is offered for trusting, a certificate shown without its
/// key least of all.
fn failure(host: &str, port: u16, pin: Option<&CertPin>, seen: Option<CertPin>, e: std::io::Error) -> NetError {
    let tls = |kind, message, fingerprint| {
        NetError::Tls(TlsError {
            kind,
            host: host.to_string(),
            message,
            certificate: None,
            fingerprint,
        })
    };
    if let (Some(pin), Some(seen)) = (pin, seen) {
        if *pin != seen {
            return tls(
                TlsErrorKind::InvalidCertificate,
                format!(
                    "the certificate at {host}:{port} changed since the first visit \
(it was SHA-256 {pin}, it is now {seen})"
                ),
                Some(seen.digest()),
            );
        }
    }
    match e.get_ref().and_then(|inner| inner.downcast_ref::<rustls::Error>()) {
        Some(rustls::Error::InvalidCertificate(_)) => tls(TlsErrorKind::InvalidCertificate, e.to_string(), None),
        Some(_) => tls(TlsErrorKind::Handshake, e.to_string(), None),
        None => socket::io(e),
    }
}

/// Accepts the certificate a first visit sees and only the pinned one after; checks the
/// handshake signatures as any verifier does.
///
/// The pin is compared once the signature proved the server holds the certificate's key:
/// rustls asks about the certificate first, and a mismatch found then would name a
/// certificate anyone can copy for the user to trust.
#[derive(Debug)]
struct TofuVerifier {
    pin: Option<CertPin>,
    algorithms: WebPkiSupportedAlgorithms,
    /// The certificate the server showed and signed for, for the error when it is not the
    /// pinned one.
    seen: Mutex<Option<CertPin>>,
}

impl TofuVerifier {
    /// `cert` signed the handshake: refuse it unless it is the pinned one.
    fn held_to_pin(&self, cert: &CertificateDer<'_>) -> Result<HandshakeSignatureValid, rustls::Error> {
        let seen = CertPin::of(cert.as_ref());
        let trusted = self.pin.as_ref().is_none_or(|pin| *pin == seen);
        *self.seen.lock() = Some(seen);
        if trusted {
            Ok(HandshakeSignatureValid::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ))
        }
    }
}

impl ServerCertVerifier for TofuVerifier {
    /// Any certificate, until it signs the handshake ([`TofuVerifier::held_to_pin`]).
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.algorithms)?;
        self.held_to_pin(cert)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.algorithms)?;
        self.held_to_pin(cert)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}
