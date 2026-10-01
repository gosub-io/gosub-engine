//! The broker↔network-process wire vocabulary.

use serde::{Deserialize, Serialize};

/// Correlates a reply with the request that caused it. Assigned by the broker;
/// the network process only echoes it back, so a confused or hostile child can
/// misroute its *own* replies and nothing else.
pub type RequestTag = u64;

/// Headers as they cross the link: one pair per value, in order, so a repeated
/// header survives. Values are raw bytes, not `String`: a value with obs-text
/// (bytes 0x80 and up, e.g. UTF-8 in `Set-Cookie` or a `Content-Disposition`
/// filename) is valid HTTP, and the in-process fetcher keeps it too.
pub type HeaderList = Vec<(String, Vec<u8>)>;

/// Flatten a header map for the link. Nothing is dropped: every
/// `HeaderValue` is valid bytes.
pub fn flatten_headers(headers: &http::HeaderMap) -> HeaderList {
    headers
        .iter()
        .map(|(n, v)| (n.as_str().to_string(), v.as_bytes().to_vec()))
        .collect()
}

/// Rebuild a header map from the link. A pair that is not a valid header
/// (only a confused or hostile peer sends one) is skipped.
pub fn rebuild_headers(headers: &HeaderList) -> http::HeaderMap {
    let mut map = http::HeaderMap::new();
    for (name, value) in headers {
        let name = http::header::HeaderName::from_bytes(name.as_bytes());
        let value = http::HeaderValue::from_bytes(value);
        if let (Ok(name), Ok(value)) = (name, value) {
            // `append`, not `insert`: each value of a repeated header is its own pair.
            map.append(name, value);
        }
    }
    map
}

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
    /// A new line to the cookie vault follows as a file descriptor (the vault
    /// was respawned); it replaces the one inherited at spawn.
    VaultLine,
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
    pub headers: HeaderList,
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

/// A random per-request capability (see [`CookieScope::ticket`]).
pub type Ticket = u128;

/// Whose cookies a request is about: the tab's zone and the document it is
/// loading, as the broker recorded them. Travels in place of the cookie
/// header when the network process asks the cookie vault itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CookieScope {
    /// A per-request capability the broker granted to the vault before
    /// dispatch; the vault answers the network process for granted tickets
    /// only, and from the grant's own scope. It covers one request: one `Get`
    /// at [`Self::url`] and one `Store`, until the broker revokes it or it
    /// expires. `0` on the broker's link.
    pub ticket: Ticket,
    /// The URL the request was granted for: under a ticket, the only one the
    /// network process may read cookies for. The `Store` names where the
    /// request ended instead, which a redirect may have moved - that much
    /// stays the network process's word, as `final_url` does.
    pub url: String,
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
        headers: HeaderList,
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
        headers: HeaderList,
        peek: Vec<u8>,
    },
    /// The request failed. A string rather than a typed error: the broker only
    /// reports it, and a rich error type would be one more thing whose
    /// deserialization a compromised child could exercise.
    Error(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_cross_the_link_with_obs_text_and_repeats_intact() {
        let mut map = http::HeaderMap::new();
        let filename =
            http::HeaderValue::from_bytes("attachment; filename=\"r\u{e9}sum\u{e9}.pdf\"".as_bytes()).unwrap();
        map.append(http::header::CONTENT_DISPOSITION, filename.clone());
        map.append(http::header::SET_COOKIE, "a=1".parse().unwrap());
        map.append(http::header::SET_COOKIE, "b=2".parse().unwrap());

        let back = rebuild_headers(&flatten_headers(&map));
        assert_eq!(back.get(http::header::CONTENT_DISPOSITION), Some(&filename));
        let cookies: Vec<_> = back.get_all(http::header::SET_COOKIE).iter().collect();
        assert_eq!(cookies, ["a=1", "b=2"]);
    }
}
