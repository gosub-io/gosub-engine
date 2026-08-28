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
    /// Abandon the request with this tag: the navigation that wanted it is gone.
    /// Best-effort - a reply already in flight simply finds no waiter.
    Cancel(RequestTag),
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
    /// A subresource of a public document: served by the strict fetcher, which
    /// refuses private-network destinations at every hop (`net::ssrf`). The
    /// broker decides this from the tab's document, never the requester.
    pub refuse_private: bool,
    /// The requester wants the body as it arrives. Honoured where the link can
    /// carry a ring fd (Linux); elsewhere the reply is buffered as usual.
    pub streaming: bool,
    /// Whose cookies to attach, when the network process has its own line to
    /// the cookie vault: the broker then sends no `Cookie` header at all, and
    /// the network process stores `Set-Cookie` in the vault itself.
    pub cookies: Option<CookieScope>,
    // Only these cross. `FetchRequest::origin` / `referrer` / `mixed_content`
    // (sonar 0.2.0) do not: the engine sets none of them yet. When it does, add
    // them here - otherwise the network process rebuilds the request without
    // them and mixed-content blocking silently disappears out-of-process.
}

/// `SameSiteContext` as it travels.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SameSite {
    SameSite,
    CrossSiteNavigation,
    CrossSite,
}

impl From<crate::engine::cookies::SameSiteContext> for SameSite {
    fn from(value: crate::engine::cookies::SameSiteContext) -> Self {
        use crate::engine::cookies::SameSiteContext as C;
        match value {
            C::SameSite => Self::SameSite,
            C::CrossSiteNavigation => Self::CrossSiteNavigation,
            C::CrossSite => Self::CrossSite,
        }
    }
}

impl From<SameSite> for crate::engine::cookies::SameSiteContext {
    fn from(value: SameSite) -> Self {
        match value {
            SameSite::SameSite => Self::SameSite,
            SameSite::CrossSiteNavigation => Self::CrossSiteNavigation,
            SameSite::CrossSite => Self::CrossSite,
        }
    }
}

/// Whose cookies a request is about: the tab's zone and the document it is
/// loading, as the broker recorded them. Travels in place of the cookie
/// header when the network process asks the cookie vault itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CookieScope {
    pub zone: String,
    pub top_level: Option<String>,
    pub samesite: SameSite,
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
    /// The response head; the body streams through a shared-memory ring
    /// (`gosub_ipc::ring`) whose fd follows this message on the link, right
    /// behind it. `peek` is what the network process had already read of the
    /// body when it answered, for content sniffing before the stream is drained.
    Streaming {
        status: u16,
        status_text: String,
        final_url: String,
        headers: Vec<(String, String)>,
        peek: Vec<u8>,
    },
    /// The request failed. A string rather than a typed error: the broker only
    /// reports it, and a rich error type would be one more thing whose
    /// deserialization a compromised child could exercise.
    Error(String),
}
