// Re-export everything from gosub_net as the canonical net implementation
pub use gosub_net::decision::decide_handling;
pub use gosub_net::decision::types::{
    BlockReason, DecisionOutcome, HandlingDecision, RenderTarget, RequestDestination,
};
pub use gosub_net::decision_hub::{DecisionHub, DecisionToken};
pub use gosub_net::shared_body::SharedBody;
pub use gosub_net::io_runtime::{spawn_io_thread, submit_to_io, IoHandle, IoContext, IoRouter};
pub use gosub_net::fetcher::{FetcherConfig, FetchInflightMap, Fetcher};
pub use gosub_net::utils::stream_to_bytes;
pub use gosub_net::net_types as types;
pub use gosub_net::req_ref_tracker;
pub use gosub_net::events;
pub use gosub_net::io_types::{IoChannel, IoCommand};

// Engine-specific extensions (not in gosub_net)
pub mod emitter;
mod router;

pub use router::{route_response_for, RoutedOutcome};
