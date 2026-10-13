//! What the network stack did to a request on the way, beyond sending it: the cache, an auth
//! challenge, an `https` upgrade, a policy that changed the request, a retry, a TLS failure.
//!
//! The engine's own copies of gosub-sonar's types, carried by the matching
//! [`ResourceEvent`](crate::engine::events::ResourceEvent) variants and by
//! [`LoadError::Tls`](crate::engine::LoadError::Tls). Mirrored rather than re-exported, like
//! [`BlockReason`](crate::net::BlockReason), so sonar can grow a case without that being a
//! breaking change here; a case this build does not know arrives as `Other`. Serialisable
//! because a TLS failure can be classified in the network process and cross to the broker.

use gosub_sonar::net::auth as sonar_auth;
use gosub_sonar::net::cache as sonar_cache;
use gosub_sonar::net::tls as sonar_tls;
use gosub_sonar::net::types as sonar_types;
use std::fmt::{Display, Formatter};

/// What the HTTP cache did with a hop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum CacheOutcome {
    /// A stored response was used without contacting the server.
    Hit,
    /// A stale stored response was used without contacting the server, as its
    /// `stale-while-revalidate` allows, and is being revalidated in the background.
    Stale,
    /// A stored response was revalidated: the server answered `304`, the stored body was used.
    Validated,
    /// The response from the network was written to the cache.
    Stored,
    /// Stored responses for the URL were dropped, because an unsafe method changed it.
    Invalidated,
}

impl CacheOutcome {
    /// Map the network layer's outcome into the engine's vocabulary.
    pub fn from_net(outcome: sonar_cache::CacheOutcome) -> Self {
        match outcome {
            sonar_cache::CacheOutcome::Hit => Self::Hit,
            sonar_cache::CacheOutcome::Stale => Self::Stale,
            sonar_cache::CacheOutcome::Validated => Self::Validated,
            sonar_cache::CacheOutcome::Stored => Self::Stored,
            sonar_cache::CacheOutcome::Invalidated => Self::Invalidated,
        }
    }
}

impl Display for CacheOutcome {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Hit => "served from cache",
            Self::Stale => "served stale from cache, revalidating",
            Self::Validated => "revalidated (304)",
            Self::Stored => "stored in cache",
            Self::Invalidated => "cache entries dropped",
        })
    }
}

/// Who asked for credentials: the origin server (`401`) or a proxy (`407`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum AuthTarget {
    /// The origin server, with `401` and `WWW-Authenticate`.
    Server,
    /// A proxy, with `407` and `Proxy-Authenticate`.
    Proxy,
}

impl AuthTarget {
    /// Map the network layer's target into the engine's vocabulary.
    pub fn from_net(target: sonar_auth::AuthTarget) -> Self {
        match target {
            sonar_auth::AuthTarget::Server => Self::Server,
            sonar_auth::AuthTarget::Proxy => Self::Proxy,
        }
    }
}

/// One challenge of a `401`/`407`: the scheme offered and the realm to show.
///
/// Only what a person needs to see. The other parameters (a Digest `nonce`, a `Negotiate`
/// token) are protocol state for answering the challenge, and stay in the network stack.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AuthChallenge {
    /// The scheme, as the protocol names it: `Basic`, `Digest`, `Bearer`, `Negotiate`, `NTLM`,
    /// or another name as the server spelled it.
    pub scheme: String,
    /// The `realm` parameter, when there was one: the protected area's name, meant to be shown.
    pub realm: Option<String>,
}

impl AuthChallenge {
    /// Keep the scheme and realm of the network layer's challenge.
    pub fn from_net(challenge: &sonar_auth::AuthChallenge) -> Self {
        Self {
            scheme: challenge.scheme.as_str().to_string(),
            realm: challenge.realm.clone(),
        }
    }
}

/// Why a hop was rewritten from `http` to `https` before it was sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum UpgradeReason {
    /// The host has a stored HSTS policy (RFC 6797).
    Hsts,
    /// An insecure subresource of a secure document was upgraded instead of blocked.
    MixedContent,
    /// A reason this build does not know.
    Other,
}

impl UpgradeReason {
    /// Map the network layer's reason into the engine's vocabulary.
    pub fn from_net(reason: sonar_types::UpgradeReason) -> Self {
        match reason {
            sonar_types::UpgradeReason::Hsts => Self::Hsts,
            sonar_types::UpgradeReason::MixedContent => Self::MixedContent,
            _ => Self::Other,
        }
    }
}

impl Display for UpgradeReason {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Hsts => "HSTS",
            Self::MixedContent => "mixed content",
            Self::Other => "another reason",
        })
    }
}

/// One way a hop differs from what was asked for, because a policy changed it instead of
/// refusing it. The request is still sent; most of these cannot be seen in the response.
///
/// URLs are as reported elsewhere: without credentials unless sensitive values are switched on.
/// Referrer policies are their header tokens, e.g. `strict-origin-when-cross-origin`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum RequestChange {
    /// A `user:password@` in the URL was removed.
    UrlCredentialsRemoved,
    /// The referrer policy trimmed or suppressed the `Referer`.
    ReferrerReduced {
        /// The referrer as it would have gone out with no policy.
        from: String,
        /// What was sent instead; `None` when no `Referer` was sent.
        to: Option<String>,
        /// The policy in force for this hop.
        policy: String,
    },
    /// `Origin` was sent as `null`: the redirect chain left the initiating origin and then moved
    /// to another one.
    OriginTainted,
    /// `Origin` was sent as `null` because the referrer policy hides the origin from this target.
    OriginHidden {
        /// The policy in force for this hop.
        policy: String,
    },
    /// Cookies and credentials were not attached, because the hop leaves the initiating origin
    /// and the request only sends them same-origin.
    CredentialsWithheld,
    /// Headers the caller set were removed because a redirect crossed origins.
    SensitiveHeadersStripped {
        /// The removed header names.
        headers: Vec<String>,
    },
    /// A redirect changed the method, and any request body was dropped with it.
    MethodDowngraded {
        /// The method of the hop that answered with the redirect.
        from: String,
        /// The method of the next hop.
        to: String,
    },
    /// A change this build does not know, as the network stack describes it.
    Other {
        /// The network stack's description.
        description: String,
    },
}

impl RequestChange {
    /// Map the network layer's change into the engine's vocabulary. `report_url` is applied to
    /// every URL it carries.
    pub fn from_net(change: &sonar_types::RequestChange, report_url: impl Fn(&url::Url) -> String) -> Self {
        use sonar_types::RequestChange as C;
        match change {
            C::UrlCredentialsRemoved => Self::UrlCredentialsRemoved,
            C::ReferrerReduced { from, to, policy } => Self::ReferrerReduced {
                from: report_url(from),
                to: to.as_ref().map(&report_url),
                policy: policy.as_str().to_string(),
            },
            C::OriginTainted => Self::OriginTainted,
            C::OriginHidden { policy } => Self::OriginHidden {
                policy: policy.as_str().to_string(),
            },
            C::CredentialsWithheld => Self::CredentialsWithheld,
            C::SensitiveHeadersStripped { headers } => Self::SensitiveHeadersStripped {
                headers: headers.iter().map(|h| h.as_str().to_string()).collect(),
            },
            C::MethodDowngraded { from, to } => Self::MethodDowngraded {
                from: from.as_str().to_string(),
                to: to.as_str().to_string(),
            },
            other => Self::Other {
                description: other.to_string(),
            },
        }
    }
}

impl Display for RequestChange {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UrlCredentialsRemoved => f.write_str("removed credentials from the URL"),
            Self::ReferrerReduced {
                to: Some(to), policy, ..
            } => {
                write!(f, "Referer reduced to {to} ({policy})")
            }
            Self::ReferrerReduced { to: None, policy, .. } => write!(f, "Referer not sent ({policy})"),
            Self::OriginTainted => f.write_str("Origin sent as null: the redirects crossed origins"),
            Self::OriginHidden { policy } => write!(f, "Origin sent as null ({policy})"),
            Self::CredentialsWithheld => f.write_str("cookies and credentials withheld: cross-origin hop"),
            Self::SensitiveHeadersStripped { headers } => {
                write!(f, "removed on a cross-origin redirect: {}", headers.join(", "))
            }
            Self::MethodDowngraded { from, to } => write!(f, "method changed from {from} to {to}, body dropped"),
            Self::Other { description } => f.write_str(description),
        }
    }
}

/// Why a TLS handshake failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum TlsFailureKind {
    /// The certificate has expired.
    Expired,
    /// The certificate is not valid yet.
    NotYetValid,
    /// The chain does not lead to a trusted root; a self-signed certificate is one.
    UnknownIssuer,
    /// The certificate is not valid for the host name that was connected to.
    HostnameMismatch,
    /// The certificate has been revoked.
    Revoked,
    /// The certificate is invalid another way: encoding, signature, algorithm, purpose.
    InvalidCertificate,
    /// The handshake failed without a certificate problem: an alert from the peer, no common
    /// protocol version or cipher suite, a malformed message.
    Handshake,
    /// Anything else.
    Other,
}

impl TlsFailureKind {
    /// Map the network layer's kind into the engine's vocabulary.
    pub fn from_net(kind: sonar_tls::TlsErrorKind) -> Self {
        use sonar_tls::TlsErrorKind as K;
        match kind {
            K::Expired => Self::Expired,
            K::NotYetValid => Self::NotYetValid,
            K::UnknownIssuer => Self::UnknownIssuer,
            K::HostnameMismatch => Self::HostnameMismatch,
            K::Revoked => Self::Revoked,
            K::InvalidCertificate => Self::InvalidCertificate,
            K::Handshake => Self::Handshake,
            _ => Self::Other,
        }
    }

    /// True when the certificate was the problem, rather than the handshake protocol.
    pub fn is_certificate_error(self) -> bool {
        matches!(
            self,
            Self::Expired
                | Self::NotYetValid
                | Self::UnknownIssuer
                | Self::HostnameMismatch
                | Self::Revoked
                | Self::InvalidCertificate
        )
    }
}

impl Display for TlsFailureKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Expired => "certificate expired",
            Self::NotYetValid => "certificate not valid yet",
            Self::UnknownIssuer => "certificate not issued by a trusted authority",
            Self::HostnameMismatch => "certificate is for another host name",
            Self::Revoked => "certificate revoked",
            Self::InvalidCertificate => "invalid certificate",
            Self::Handshake => "handshake failed",
            Self::Other => "TLS failure",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> url::Url {
        url::Url::parse(s).unwrap()
    }

    #[test]
    fn every_tls_kind_sonar_names_has_its_own_engine_kind() {
        use sonar_tls::TlsErrorKind as K;
        let pairs = [
            (K::Expired, TlsFailureKind::Expired),
            (K::NotYetValid, TlsFailureKind::NotYetValid),
            (K::UnknownIssuer, TlsFailureKind::UnknownIssuer),
            (K::HostnameMismatch, TlsFailureKind::HostnameMismatch),
            (K::Revoked, TlsFailureKind::Revoked),
            (K::InvalidCertificate, TlsFailureKind::InvalidCertificate),
            (K::Handshake, TlsFailureKind::Handshake),
            (K::Other, TlsFailureKind::Other),
        ];
        for (net, engine) in pairs {
            assert_eq!(TlsFailureKind::from_net(net), engine, "{net:?}");
        }
    }

    /// The line a "continue anyway" button is drawn on: a certificate a person can choose to
    /// trust, not a handshake that could not happen at all. It has to agree with sonar's,
    /// because sonar is what then accepts the override.
    #[test]
    fn only_certificate_problems_are_certificate_errors() {
        use sonar_tls::TlsErrorKind as K;
        let kinds = [
            K::Expired,
            K::NotYetValid,
            K::UnknownIssuer,
            K::HostnameMismatch,
            K::Revoked,
            K::InvalidCertificate,
            K::Handshake,
            K::Other,
        ];
        for kind in kinds {
            assert_eq!(
                TlsFailureKind::from_net(kind).is_certificate_error(),
                kind.is_certificate_error(),
                "{kind:?}"
            );
        }
        assert!(!TlsFailureKind::Handshake.is_certificate_error());
        assert!(!TlsFailureKind::Other.is_certificate_error());
    }

    /// Policies travel as their header tokens, which is what a person reading a network panel
    /// recognises and what they would type into a `Referrer-Policy` themselves.
    #[test]
    fn a_reduced_referrer_keeps_its_policy_token_and_passes_its_urls_through_report_url() {
        let change = sonar_types::RequestChange::ReferrerReduced {
            from: url("https://example.test/page?q=1"),
            to: Some(url("https://example.test/")),
            policy: gosub_sonar::net::referrer::ReferrerPolicy::StrictOriginWhenCrossOrigin,
        };
        let mapped = RequestChange::from_net(&change, |u| format!("reported:{u}"));
        assert_eq!(
            mapped,
            RequestChange::ReferrerReduced {
                from: "reported:https://example.test/page?q=1".into(),
                to: Some("reported:https://example.test/".into()),
                policy: "strict-origin-when-cross-origin".into(),
            }
        );

        let suppressed = sonar_types::RequestChange::ReferrerReduced {
            from: url("https://example.test/"),
            to: None,
            policy: gosub_sonar::net::referrer::ReferrerPolicy::NoReferrer,
        };
        let RequestChange::ReferrerReduced { to, policy, .. } = RequestChange::from_net(&suppressed, |u| u.to_string())
        else {
            panic!("expected ReferrerReduced");
        };
        assert_eq!(to, None);
        assert_eq!(policy, "no-referrer");
    }

    #[test]
    fn a_hidden_origin_keeps_its_policy_token() {
        let change = sonar_types::RequestChange::OriginHidden {
            policy: gosub_sonar::net::referrer::ReferrerPolicy::SameOrigin,
        };
        assert_eq!(
            RequestChange::from_net(&change, |u| u.to_string()),
            RequestChange::OriginHidden {
                policy: "same-origin".into()
            }
        );
    }

    #[test]
    fn stripped_headers_are_reported_by_name_and_a_downgrade_by_method() {
        let stripped = sonar_types::RequestChange::SensitiveHeadersStripped {
            headers: vec![http::header::AUTHORIZATION, http::header::COOKIE],
        };
        assert_eq!(
            RequestChange::from_net(&stripped, |u| u.to_string()),
            RequestChange::SensitiveHeadersStripped {
                headers: vec!["authorization".into(), "cookie".into()]
            }
        );

        let downgraded = sonar_types::RequestChange::MethodDowngraded {
            from: http::Method::POST,
            to: http::Method::GET,
        };
        assert_eq!(
            RequestChange::from_net(&downgraded, |u| u.to_string()),
            RequestChange::MethodDowngraded {
                from: "POST".into(),
                to: "GET".into()
            }
        );
    }

    #[test]
    fn changes_without_detail_map_one_to_one() {
        use sonar_types::RequestChange as C;
        let pairs = [
            (C::UrlCredentialsRemoved, RequestChange::UrlCredentialsRemoved),
            (C::OriginTainted, RequestChange::OriginTainted),
            (C::CredentialsWithheld, RequestChange::CredentialsWithheld),
        ];
        for (net, engine) in pairs {
            assert_eq!(RequestChange::from_net(&net, |u| u.to_string()), engine);
        }
    }

    /// A Digest `nonce` or a `Negotiate` token is protocol state for answering the challenge.
    /// Nothing outside the network stack needs it, so it does not leave.
    #[test]
    fn an_auth_challenge_keeps_the_scheme_and_realm_and_nothing_else() {
        let challenge = sonar_auth::AuthChallenge {
            url: url("https://example.test/private/"),
            target: sonar_auth::AuthTarget::Server,
            scheme: sonar_auth::AuthScheme::Digest,
            realm: Some("staff".into()),
            params: vec![
                ("realm".into(), "staff".into()),
                ("nonce".into(), "dcd98b7102dd2f0e".into()),
            ],
            token68: Some("c2VjcmV0".into()),
            attempt: 1,
            proxy: None,
        };
        assert_eq!(
            AuthChallenge::from_net(&challenge),
            AuthChallenge {
                scheme: "Digest".into(),
                realm: Some("staff".into()),
            }
        );

        // A scheme sonar does not know keeps the server's spelling.
        let other = sonar_auth::AuthChallenge {
            scheme: sonar_auth::AuthScheme::Other("Mutual".into()),
            realm: None,
            ..challenge
        };
        assert_eq!(
            AuthChallenge::from_net(&other),
            AuthChallenge {
                scheme: "Mutual".into(),
                realm: None,
            }
        );
    }
}
