use crate::net::types::NetError;
use crate::net::BlockReason;
use gosub_sonar::TransportErrorKind;
/// Public engine errors available for the outside world
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("Invalid tab ID")]
    InvalidTabId,

    #[error("Invalid zone ID")]
    InvalidZoneId,

    #[error("Zone limit exceeded")]
    ZoneLimitExceeded,

    #[error("Network error: {0}")]
    NetworkError(String),

    #[error("Parser error: {0}")]
    ParserError(String),

    #[error("Renderer error: {0}")]
    RendererError(String),

    #[error("Internal engine error: {0}")]
    Internal(#[source] anyhow::Error),

    /// The zone provided by the zone id is not found (permissions or does not exist)
    #[error("Zone not found")]
    ZoneNotFound,

    #[error("Zone is already locked")]
    ZoneLocked,

    #[error("Tab limit in zone exceeded")]
    TabLimitExceeded,

    #[error("Zone already exists")]
    ZoneAlreadyExists,

    #[error("Invalid configuration: {0}")]
    InvalidConfiguration(String),

    #[error("Task init failed: {0}")]
    TaskInitFailed(#[source] anyhow::Error),

    #[error("Failed to create tab: {0}")]
    CreateTab(#[source] anyhow::Error),

    #[error("Channel closed")]
    ChannelClosed,

    #[error("Failed to create zone: {0}")]
    CreateZone(#[source] anyhow::Error),

    #[error("Engine is already running")]
    AlreadyRunning,

    #[error("Engine is not running")]
    NotRunning,

    #[error("I/O runtime not started")]
    IoNotStarted,

    /// A cookie/storage backing store failed to initialize.
    #[error("Cookie store error: {0}")]
    CookieStore(#[source] anyhow::Error),
}

#[derive(thiserror::Error, Debug)]
pub enum NavigationError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("network error: {0}")]
    NetworkError(String),

    /// A failure the network layer classified for us. Kept typed rather than stringified so
    /// the classification survives all the way to [`LoadError`].
    #[error(transparent)]
    Net(NetError),

    #[error("io cancelled: {0}")]
    Cancelled(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Why a navigation or a resource load failed.
///
/// Match on the variant to decide what to show and whether retrying could help;
/// [`Display`](std::fmt::Display) gives the message to show. Derived from the network
/// stack's own typed error rather than by reading its message, so it says only what is
/// known: a name that did not resolve and a server that refused the connection both arrive
/// as [`Connect`](Self::Connect), because the client does not separate them.
///
/// `#[non_exhaustive]`: whatever the network stack learns to tell apart next lands as a
/// new variant without a breaking change.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LoadError {
    /// Refused by policy before it was sent - mixed content, URL policy, CORS. Retrying
    /// will not help.
    Blocked {
        /// What refused it.
        reason: BlockReason,
    },
    /// The URL string could not be parsed.
    InvalidUrl {
        /// What was wrong with it.
        message: String,
    },
    /// No connection was established: the name did not resolve, or nothing accepted it.
    Connect {
        /// The underlying failure, as reported.
        message: String,
    },
    /// The TLS handshake failed: an expired, untrusted or mismatched certificate.
    Tls {
        /// The underlying failure, as reported.
        message: String,
    },
    /// The request did not complete within the configured time limit.
    Timeout {
        /// What timed out.
        message: String,
    },
    /// The connection worked and then broke part way through the transfer.
    Transfer {
        /// The underlying failure, as reported.
        message: String,
    },
    /// A redirect could not be followed: too many hops, or an invalid target.
    Redirect {
        /// The underlying failure, as reported.
        message: String,
    },
    /// A local I/O failure - writing a download, reading a spooled body, opening storage.
    Io {
        /// The underlying failure, as reported.
        message: String,
    },
    /// The load was cancelled: a new navigation, the tab closing, or an explicit cancel.
    Cancelled {
        /// Why it was cancelled.
        message: String,
    },
    /// The bytes arrived but could not be turned into a document.
    Content {
        /// What went wrong.
        message: String,
    },
    /// Anything the engine cannot classify.
    Other {
        /// What went wrong.
        message: String,
    },
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Blocked { reason } => write!(f, "{reason}"),
            LoadError::InvalidUrl { message } => write!(f, "invalid URL: {message}"),
            LoadError::Connect { message } => write!(f, "could not connect: {message}"),
            LoadError::Tls { message } => write!(f, "TLS error: {message}"),
            LoadError::Timeout { message } => write!(f, "timed out: {message}"),
            LoadError::Transfer { message } => write!(f, "transfer failed: {message}"),
            LoadError::Redirect { message } => write!(f, "bad redirect: {message}"),
            LoadError::Io { message } => write!(f, "I/O error: {message}"),
            LoadError::Cancelled { message } => write!(f, "cancelled: {message}"),
            LoadError::Content { message } => write!(f, "content error: {message}"),
            LoadError::Other { message } => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for LoadError {}

impl From<&NetError> for LoadError {
    fn from(e: &NetError) -> Self {
        match e {
            NetError::Blocked { reason, .. } => LoadError::Blocked {
                reason: BlockReason::from_net(*reason),
            },
            // Sonar has already separated these. A `send()` that never got a connection
            // and a body that stopped mid-stream are both transport failures, and
            // reporting the first as a broken transfer sends you looking at the server
            // when the problem is the address.
            NetError::Transport(t) => {
                let message = t.message.clone();
                match t.kind {
                    TransportErrorKind::Connect => LoadError::Connect { message },
                    TransportErrorKind::Timeout => LoadError::Timeout { message },
                    TransportErrorKind::Redirect => LoadError::Redirect { message },
                    TransportErrorKind::Body | TransportErrorKind::Decode => LoadError::Transfer { message },
                    // `Request` and `Builder` mean nothing was sent, and whatever sonar
                    // learns to tell apart later lands here first. Neither says anything
                    // about the network.
                    _ => LoadError::Other { message },
                }
            }
            NetError::Timeout(message) => LoadError::Timeout {
                message: message.clone(),
            },
            NetError::Tls(tls) => LoadError::Tls {
                message: tls.to_string(),
            },
            NetError::Redirect(err) => LoadError::Redirect {
                message: format!("{err:#}"),
            },
            NetError::Io(err) => LoadError::Transfer {
                message: err.to_string(),
            },
            NetError::Cancelled(message) => LoadError::Cancelled {
                message: message.clone(),
            },
            NetError::Read(err) => LoadError::Transfer {
                message: format!("{err:#}"),
            },
            NetError::Other(err) => LoadError::Other {
                message: format!("{err:#}"),
            },
        }
    }
}

/// Read the network stack's own typed error out of an `anyhow` wrapper.
///
/// The `NetError` underneath is intact and already says what went wrong, including when a
/// caller attached context on the way. Anything else says nothing about the cause, and
/// claiming one would be worse than admitting we do not know.
impl From<&anyhow::Error> for LoadError {
    fn from(error: &anyhow::Error) -> Self {
        match error.downcast_ref::<NetError>() {
            Some(net) => LoadError::from(net),
            None => LoadError::Other {
                message: format!("{error:#}"),
            },
        }
    }
}

impl From<NavigationError> for LoadError {
    fn from(e: NavigationError) -> Self {
        match e {
            NavigationError::Io(err) => LoadError::Io {
                message: err.to_string(),
            },
            // A string from the engine's own plumbing (a closed channel, a routing
            // failure); nothing typed to classify it by.
            NavigationError::NetworkError(message) => LoadError::Other { message },
            NavigationError::Net(ref e) => LoadError::from(e),
            NavigationError::Cancelled(message) => LoadError::Cancelled { message },
            NavigationError::Other(ref err) => LoadError::from(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gosub_sonar::TransportError;

    /// A policy refusal and a transport failure must be distinguishable without matching
    /// on error strings.
    #[test]
    fn kinds_are_distinguishable_without_parsing_messages() {
        let blocked = LoadError::Blocked {
            reason: BlockReason::MixedContent,
        };
        let connect = LoadError::Connect {
            message: "connection refused".into(),
        };
        assert!(matches!(blocked, LoadError::Blocked { .. }));
        assert!(matches!(connect, LoadError::Connect { .. }));
        assert_ne!(blocked, connect);
    }

    /// Code that only prints the error keeps working, which is why the swap from
    /// `Arc<anyhow::Error>` did not need consumer changes.
    #[test]
    fn display_carries_a_readable_message() {
        assert_eq!(
            LoadError::Blocked {
                reason: BlockReason::UnsupportedScheme
            }
            .to_string(),
            "unsupported URL scheme"
        );
        assert_eq!(
            LoadError::Connect {
                message: "dns failure".into()
            }
            .to_string(),
            "could not connect: dns failure"
        );
        assert_eq!(
            LoadError::InvalidUrl {
                message: "relative URL without a base".into()
            }
            .to_string(),
            "invalid URL: relative URL without a base"
        );
    }

    /// The engine's internal `NavigationError` is already typed; the conversion must keep
    /// that classification rather than collapsing everything into `Other`.
    #[test]
    fn navigation_error_keeps_its_classification() {
        assert!(matches!(
            LoadError::from(NavigationError::Cancelled("new navigation".into())),
            LoadError::Cancelled { .. }
        ));
        assert!(matches!(
            LoadError::from(NavigationError::Io(std::io::Error::other("disk"))),
            LoadError::Io { .. }
        ));
        assert!(matches!(
            LoadError::from(NavigationError::Other(anyhow::anyhow!("odd"))),
            LoadError::Other { .. }
        ));
        // A typed network error wrapped in anyhow on the way keeps its kind.
        let wrapped = anyhow::Error::from(NetError::Timeout("took too long".into())).context("loading page");
        assert!(matches!(
            LoadError::from(NavigationError::Other(wrapped)),
            LoadError::Timeout { .. }
        ));
    }

    /// The router hands fetch failures on as `anyhow`, so the classification only survives
    /// if `NetError` can be recovered by downcast. If anyhow ever stopped preserving the
    /// concrete type, every navigation failure would silently collapse to `Other`.
    #[test]
    fn net_error_survives_the_anyhow_round_trip() {
        let original = NetError::Timeout("took too long".into());
        let wrapped = anyhow::anyhow!(original);

        let recovered = wrapped
            .downcast_ref::<NetError>()
            .expect("NetError must be recoverable");
        assert!(matches!(LoadError::from(recovered), LoadError::Timeout { .. }));
    }

    /// Each network failure the engine can be handed must keep its kind.
    #[test]
    fn net_error_kinds_map_across() {
        use gosub_sonar::net::types::BlockReason as Net;

        let cases = [
            (
                NetError::Blocked {
                    reason: Net::MixedContent,
                    url: url::Url::parse("http://example.org/").unwrap(),
                },
                LoadError::Blocked {
                    reason: BlockReason::MixedContent,
                },
            ),
            (
                NetError::Timeout("t".into()),
                LoadError::Timeout { message: "t".into() },
            ),
            (
                NetError::Cancelled("c".into()),
                LoadError::Cancelled { message: "c".into() },
            ),
        ];
        for (net, expected) in cases {
            assert_eq!(LoadError::from(&net), expected, "mapping {net:?}");
        }
        // A transfer that broke keeps its kind even though the message is the OS's.
        assert!(matches!(
            LoadError::from(&NetError::from(std::io::Error::other("disk"))),
            LoadError::Transfer { .. }
        ));
    }

    /// The case that made the split worth doing: a host nothing is listening on and a body
    /// that stopped mid-stream are both transport failures. Reported as a broken transfer
    /// the first sends you looking at the server; reported as a connection failure it sends
    /// you at the address, which is where the problem is. Sonar draws the line and tests it
    /// against a real socket; this checks the engine keeps it.
    #[test]
    fn a_host_that_never_connects_is_not_a_broken_transfer() {
        let connect = NetError::Transport(TransportError {
            kind: TransportErrorKind::Connect,
            message: "connection refused".into(),
        });
        assert!(matches!(LoadError::from(&connect), LoadError::Connect { .. }));

        let mid_body = NetError::Transport(TransportError {
            kind: TransportErrorKind::Body,
            message: "error reading a body from connection".into(),
        });
        assert!(matches!(LoadError::from(&mid_body), LoadError::Transfer { .. }));
    }

    /// `TransportErrorKind` is non-exhaustive, so the catch-all arm gets whatever sonar
    /// learns to tell apart next. It must not be reported as a kind the engine does know.
    #[test]
    fn an_unmapped_transport_kind_claims_nothing() {
        let err = NetError::Transport(TransportError {
            kind: TransportErrorKind::Builder,
            message: "invalid header value".into(),
        });
        assert!(matches!(LoadError::from(&err), LoadError::Other { .. }));
    }

    /// An error from somewhere other than the network stack says nothing about the cause.
    #[test]
    fn an_unrecognised_error_claims_nothing() {
        assert!(matches!(
            LoadError::from(&anyhow::anyhow!("something went wrong")),
            LoadError::Other { .. }
        ));
    }

    /// Every refusal the network layer can report must map to a distinct engine reason.
    #[test]
    fn every_net_block_reason_maps_to_a_distinct_reason() {
        use gosub_sonar::net::types::BlockReason as Net;
        let mapped = [Net::MixedContent, Net::UrlPolicy, Net::UnsupportedScheme].map(BlockReason::from_net);
        assert_eq!(
            mapped,
            [
                BlockReason::MixedContent,
                BlockReason::UrlPolicy,
                BlockReason::UnsupportedScheme
            ]
        );
    }
}
