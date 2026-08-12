//! The broker↔network-process wire vocabulary.

use serde::{Deserialize, Serialize};

/// Correlates a reply with the request that caused it. Assigned by the broker;
/// the network process only echoes it back, so a confused or hostile child can
/// misroute its *own* replies and nothing else.
pub type RequestTag = u64;

/// Broker → network process.
#[derive(Debug, Serialize, Deserialize)]
pub enum ToNet {
    /// Prove the child is a network process before anything is entrusted to it.
    Ping,
    Fetch(NetFetch),
    /// Finish in-flight work and exit. The broker still waits for the process to
    /// go away and kills it if it does not.
    Shutdown,
}

/// One request, flattened to what actually has to travel.
#[derive(Debug, Serialize, Deserialize)]
pub struct NetFetch {
    pub tag: RequestTag,
    pub url: String,
    pub method: String,
    /// Includes the `Cookie` header the broker attached (see
    /// [`crate::net::tab_identity`]). The network process is trusted with cookie
    /// *values* because it must put them on the wire; the renderer is not, and
    /// that is the boundary this whole exercise is about. A cookie vault that
    /// keeps values out of the broker→net hop too is a later refinement - see
    /// the PoC's `vault` component.
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
}

/// Network process → broker.
#[derive(Debug, Serialize, Deserialize)]
pub enum FromNet {
    /// Answer to [`ToNet::Ping`]: this really is a network process.
    Pong,
    Reply {
        tag: RequestTag,
        outcome: FetchOutcome,
    },
}

/// What became of a request.
#[derive(Debug, Serialize, Deserialize)]
pub enum FetchOutcome {
    Ok {
        status: u16,
        status_text: String,
        /// After redirects - the broker needs this to attribute `Set-Cookie`
        /// correctly, so it must come from the process that followed them.
        final_url: String,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    },
    /// The request failed. A string rather than a typed error: the broker only
    /// reports it, and a rich error type would be one more thing whose
    /// deserialization a compromised child could exercise.
    Error(String),
}
