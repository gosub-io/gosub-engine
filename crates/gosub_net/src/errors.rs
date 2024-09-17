//! Error results that can be returned from the Gosub net module
use thiserror::Error;

/// Serious errors and errors from third-party libraries
#[derive(Debug, Error)]
pub enum Error {
    #[error("ureq error")]
    Request(#[from] Box<crate::http::HttpError>),

    #[error("io error: {0}")]
    IO(#[from] std::io::Error),

    #[error("dns: generic error: {0}")]
    DnsGeneric(String),

    #[error("dns: no ipv6 address found")]
    DnsNoIpv6Found,

    #[error("dns: no ipv4 address found")]
    DnsNoIpv4Found,

    #[error("dns: no ip address found")]
    DnsNoIpAddressFound,

    #[error("dns: domain not found")]
    DnsDomainNotFound,

    #[error("there was a problem: {0}")]
    Generic(String),

    #[error("failed to parse url: {0}")]
    Url(#[from] url::ParseError),
}
