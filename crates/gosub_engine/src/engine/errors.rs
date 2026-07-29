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

    #[error("io cancelled: {0}")]
    Cancelled(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
