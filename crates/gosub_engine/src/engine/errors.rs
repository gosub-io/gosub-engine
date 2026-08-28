use crate::net::types::NetError;
use crate::net::BlockReason;
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
/// [`Display`](std::fmt::Display) gives the message to show.
///
/// `#[non_exhaustive]` because [`Network`](Self::Network) is coarser than it should be - the
/// HTTP client reports DNS, connect and TLS failures as one error - and splitting it later
/// must not be breaking.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LoadError {
    /// Refused before or instead of loading. Retrying will not help.
    Blocked {
        /// What refused it.
        reason: BlockReason,
    },
    /// The URL string could not be parsed.
    InvalidUrl {
        /// What was wrong with it.
        message: String,
    },
    /// The transfer failed: DNS, connection, TLS or HTTP. One bucket because the HTTP client
    /// does not separate them for us; [`Timeout`](Self::Timeout) is split out because the
    /// network layer does report that distinctly.
    Network {
        /// The underlying failure, as reported.
        message: String,
    },
    /// The request did not complete within the configured time limit.
    Timeout {
        /// What timed out.
        message: String,
    },
    /// A local I/O failure - writing a download, reading a body, opening storage.
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
            LoadError::Network { message } => write!(f, "network error: {message}"),
            LoadError::Timeout { message } => write!(f, "timed out: {message}"),
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
            NetError::Timeout(message) => LoadError::Timeout {
                message: message.clone(),
            },
            NetError::Cancelled(message) => LoadError::Cancelled {
                message: message.clone(),
            },
            NetError::Io(err) => LoadError::Io {
                message: err.to_string(),
            },
            // reqwest folds DNS, connect, TLS and protocol failures together, and the
            // redirect/read errors are transport failures too.
            NetError::Reqwest(_) | NetError::Redirect(_) | NetError::Read(_) => {
                LoadError::Network { message: e.to_string() }
            }
            NetError::Other(err) => LoadError::Other {
                message: format!("{err:#}"),
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
            NavigationError::NetworkError(message) => LoadError::Network { message },
            NavigationError::Net(ref e) => LoadError::from(e),
            NavigationError::Cancelled(message) => LoadError::Cancelled { message },
            NavigationError::Other(err) => LoadError::Other {
                message: format!("{err:#}"),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A policy refusal and a transport failure must be distinguishable without matching
    /// on error strings.
    #[test]
    fn kinds_are_distinguishable_without_parsing_messages() {
        let blocked = LoadError::Blocked {
            reason: BlockReason::MixedContent,
        };
        let network = LoadError::Network {
            message: "connection refused".into(),
        };
        assert!(matches!(blocked, LoadError::Blocked { .. }));
        assert!(matches!(network, LoadError::Network { .. }));
        assert_ne!(blocked, network);
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
            LoadError::Network {
                message: "dns failure".into()
            }
            .to_string(),
            "network error: dns failure"
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
            LoadError::from(NavigationError::NetworkError("boom".into())),
            LoadError::Network { .. }
        ));
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
    }

    /// The router hands fetch failures on as `anyhow`, so the classification only survives
    /// if `NetError` can be recovered by downcast. If anyhow ever stopped preserving the
    /// concrete type, every navigation failure would silently collapse to `Network`.
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
        // I/O keeps its kind even though the message is the OS's.
        assert!(matches!(
            LoadError::from(&NetError::from(std::io::Error::other("disk"))),
            LoadError::Io { .. }
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
