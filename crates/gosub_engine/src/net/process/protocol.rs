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
    /// Boxed: a request is the one large message on this link, and the rest
    /// should not be sized for it.
    Fetch(Box<NetFetch>),
    /// Abandon the request with this tag: the navigation that wanted it is gone.
    /// Best-effort - a reply already in flight simply finds no waiter.
    Cancel(RequestTag),
    /// Finish in-flight work and exit. The broker still waits for the process to
    /// go away and kills it if it does not.
    Shutdown,
    /// A new line to the cookie vault follows as a file descriptor (the vault
    /// was respawned); it replaces the one inherited at spawn.
    VaultLine,
    /// Run the escape audit under the net lockdown and report it; the reply
    /// echoes `tag`, so a late answer is never taken for a later request's.
    Audit { tag: RequestTag },
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
    /// How much of the response body the broker wants previewed, in bytes;
    /// `None` captures nothing. The broker's switches decide this (see
    /// `crate::net::emitter`); this process has no settings of its own.
    #[serde(default)]
    pub body_preview: Option<usize>,
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
    /// Answer to [`ToNet::Audit`]; `None` where the audit cannot run.
    Audit {
        tag: RequestTag,
        report: Option<gosub_sandbox::audit::AuditReport>,
    },
    /// Something happened to the request with this tag - a name resolved, a
    /// connection opened, headers in, bytes received, the end. What the
    /// in-process fetcher would have told its observer; the broker's observer
    /// for the request gets it instead. For a streamed body these keep
    /// arriving after the `Reply` that carried the head.
    Event {
        tag: RequestTag,
        event: NetEventWire,
    },
}

/// Longest string a network process may put in an event - a URL, a host, a
/// method, a reason. Cut on receipt.
pub const MAX_EVENT_STRING: usize = 8 * 1024;
/// Most headers an event may carry; the rest are dropped on receipt.
pub const MAX_EVENT_HEADERS: usize = 256;

/// A network event as it travels: the ones the engine reports to the embedder
/// (see `EngineEventEmitter`), flattened to plain data. Durations in
/// microseconds, URLs as strings, a failure already classified where the
/// typed error exists - the string the broker would otherwise receive could
/// not be classified again.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NetEventWire {
    DnsResolved {
        host: String,
        elapsed_us: u64,
        addr_count: u64,
    },
    Connected {
        elapsed_us: u64,
    },
    RequestSent {
        url: String,
        method: String,
        headers: HeaderList,
    },
    BodyPreview {
        url: String,
        body: Vec<u8>,
        truncated: bool,
    },
    Started {
        url: String,
    },
    Redirected {
        from: String,
        to: String,
        status: u16,
    },
    ResponseHeaders {
        url: String,
        status: u16,
        headers: HeaderList,
    },
    Progress {
        received_bytes: u64,
        expected_length: Option<u64>,
        elapsed_us: u64,
    },
    Finished {
        received_bytes: u64,
        elapsed_us: u64,
        url: String,
    },
    Failed {
        url: String,
        error: crate::engine::LoadError,
    },
    Cancelled {
        url: String,
        reason: String,
    },
}

/// What the broker says a request cancelled in the network process was cancelled for.
pub const CANCELLED_IN_NET: &str = "cancelled in the network process";

impl NetEventWire {
    /// The wire form of `event`, for the events the engine reports; `None`
    /// for the ones it only logs.
    pub fn from_net(event: &crate::net::events::NetEvent) -> Option<Self> {
        use crate::engine::LoadError;
        use crate::net::events::NetEvent;
        let us = |d: &std::time::Duration| d.as_micros() as u64;
        Some(match event {
            NetEvent::DnsResolved {
                host,
                elapsed,
                addr_count,
            } => Self::DnsResolved {
                host: host.clone(),
                elapsed_us: us(elapsed),
                addr_count: *addr_count as u64,
            },
            NetEvent::Connected { elapsed } => Self::Connected {
                elapsed_us: us(elapsed),
            },
            NetEvent::RequestSent { url, method, headers } => Self::RequestSent {
                url: url.to_string(),
                method: method.as_str().to_string(),
                headers: flatten_headers(headers),
            },
            NetEvent::BodyPreview { url, body, truncated } => Self::BodyPreview {
                url: url.to_string(),
                body: body.clone(),
                truncated: *truncated,
            },
            NetEvent::Started { url } => Self::Started { url: url.to_string() },
            NetEvent::Redirected { from, to, status } => Self::Redirected {
                from: from.to_string(),
                to: to.to_string(),
                status: *status,
            },
            NetEvent::ResponseHeaders { url, status, headers } => Self::ResponseHeaders {
                url: url.to_string(),
                status: *status,
                headers: flatten_headers(headers),
            },
            NetEvent::Progress {
                received_bytes,
                expected_length,
                elapsed,
            } => Self::Progress {
                received_bytes: *received_bytes,
                expected_length: *expected_length,
                elapsed_us: us(elapsed),
            },
            NetEvent::Finished {
                received_bytes,
                elapsed,
                url,
            } => Self::Finished {
                received_bytes: *received_bytes,
                elapsed_us: us(elapsed),
                url: url.to_string(),
            },
            // The three failures the emitter reports, classified here where
            // the typed cause still exists.
            NetEvent::Failed { url, error } => Self::Failed {
                url: url.to_string(),
                error: LoadError::from(error),
            },
            NetEvent::Blocked { url, reason } => Self::Failed {
                url: url.to_string(),
                error: LoadError::Blocked {
                    reason: crate::net::BlockReason::from_net(*reason),
                },
            },
            NetEvent::TlsFailed { url, error } => Self::Failed {
                url: url.to_string(),
                error: LoadError::Tls {
                    message: format!(
                        "TLS handshake with {} failed: {:?} ({})",
                        error.host, error.kind, error.message
                    ),
                },
            },
            NetEvent::Cancelled { url, reason } => Self::Cancelled {
                url: url.to_string(),
                reason: (*reason).to_string(),
            },
            _ => return None,
        })
    }

    /// Whether this is the last event a request produces.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Finished { .. } | Self::Failed { .. } | Self::Cancelled { .. }
        )
    }

    /// Back into a network event, for the broker's own observer of the
    /// request. Everything here came from a child: strings are cut to
    /// [`MAX_EVENT_STRING`], headers to [`MAX_EVENT_HEADERS`], a body preview
    /// to `preview_cap` bytes, and an event whose URL does not parse is dropped.
    pub fn into_net(self, preview_cap: usize) -> Option<crate::net::events::NetEvent> {
        use crate::net::events::NetEvent;
        use std::time::Duration;
        let cut = |s: String| -> String {
            if s.len() <= MAX_EVENT_STRING {
                return s;
            }
            let mut end = MAX_EVENT_STRING;
            while !s.is_char_boundary(end) {
                end -= 1;
            }
            s[..end].to_string()
        };
        let url = |s: String| url::Url::parse(&cut(s)).ok();
        let headers = |mut list: HeaderList| {
            list.truncate(MAX_EVENT_HEADERS);
            list.retain(|(name, value)| name.len() <= MAX_EVENT_STRING && value.len() <= MAX_EVENT_STRING);
            rebuild_headers(&list)
        };
        Some(match self {
            Self::DnsResolved {
                host,
                elapsed_us,
                addr_count,
            } => NetEvent::DnsResolved {
                host: cut(host),
                elapsed: Duration::from_micros(elapsed_us),
                addr_count: addr_count as usize,
            },
            Self::Connected { elapsed_us } => NetEvent::Connected {
                elapsed: Duration::from_micros(elapsed_us),
            },
            Self::RequestSent {
                url: u,
                method,
                headers: h,
            } => NetEvent::RequestSent {
                url: url(u)?,
                method: http::Method::from_bytes(cut(method).as_bytes()).ok()?,
                headers: headers(h),
            },
            Self::BodyPreview {
                url: u,
                mut body,
                truncated,
            } => {
                let clipped = body.len() > preview_cap;
                body.truncate(preview_cap);
                NetEvent::BodyPreview {
                    url: url(u)?,
                    body,
                    truncated: truncated || clipped,
                }
            }
            Self::Started { url: u } => NetEvent::Started { url: url(u)? },
            Self::Redirected { from, to, status } => NetEvent::Redirected {
                from: url(from)?,
                to: url(to)?,
                status,
            },
            Self::ResponseHeaders {
                url: u,
                status,
                headers: h,
            } => NetEvent::ResponseHeaders {
                url: url(u)?,
                status,
                headers: headers(h),
            },
            Self::Progress {
                received_bytes,
                expected_length,
                elapsed_us,
            } => NetEvent::Progress {
                received_bytes,
                expected_length,
                elapsed: Duration::from_micros(elapsed_us),
            },
            Self::Finished {
                received_bytes,
                elapsed_us,
                url: u,
            } => NetEvent::Finished {
                received_bytes,
                elapsed: Duration::from_micros(elapsed_us),
                url: url(u)?,
            },
            // The classification travels inside the error; `LoadError::from`
            // finds it again on the other side. Its message is the child's.
            Self::Failed { url: u, error } => NetEvent::Failed {
                url: url(u)?,
                error: anyhow::Error::new(error.map_message(cut)),
            },
            Self::Cancelled { url: u, .. } => NetEvent::Cancelled {
                url: url(u)?,
                reason: CANCELLED_IN_NET,
            },
        })
    }
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
        /// Where the response came from (`FetchResultMeta::peer_addr`): what the
        /// broker places the document by on the private-network policy.
        peer_addr: Option<std::net::SocketAddr>,
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
        peer_addr: Option<std::net::SocketAddr>,
    },
    /// The request failed. A string rather than a typed error: the broker only
    /// reports it, and a rich error type would be one more thing whose
    /// deserialization a compromised child could exercise.
    Error(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A failure classified in the network process reaches the broker's
    /// observer classified, and a hostile preview is clipped to what was asked.
    #[test]
    fn events_survive_the_link_and_are_bounded_on_receipt() {
        use crate::engine::LoadError;
        use crate::net::events::NetEvent;
        let failed = NetEventWire::Failed {
            url: "https://site.test/a.css".into(),
            error: LoadError::Blocked {
                reason: crate::net::BlockReason::MixedContent,
            },
        };
        let bytes = serde_json::to_vec(&failed).unwrap();
        let back: NetEventWire = serde_json::from_slice(&bytes).unwrap();
        assert!(back.is_terminal());
        match back.into_net(1024).unwrap() {
            NetEvent::Failed { error, .. } => {
                assert!(matches!(LoadError::from(&error), LoadError::Blocked { .. }), "{error}")
            }
            other => panic!("not a failure: {other:?}"),
        }

        let preview = NetEventWire::BodyPreview {
            url: "https://site.test/".into(),
            body: vec![7u8; 10_000],
            truncated: false,
        };
        match preview.into_net(100).unwrap() {
            NetEvent::BodyPreview { body, truncated, .. } => {
                assert_eq!(body.len(), 100);
                assert!(truncated);
            }
            other => panic!("not a preview: {other:?}"),
        }

        let huge = NetEventWire::Started {
            url: format!("https://site.test/{}", "a".repeat(MAX_EVENT_STRING)),
        };
        // Cut to the limit, which may leave a URL that still parses: either way
        // nothing longer than the limit reaches the engine.
        if let Some(NetEvent::Started { url }) = huge.into_net(0) {
            assert!(url.as_str().len() <= MAX_EVENT_STRING);
        }
        assert!(NetEventWire::Started {
            url: "not a url".into()
        }
        .into_net(0)
        .is_none());

        let loud = NetEventWire::Failed {
            url: "https://site.test/".into(),
            error: crate::LoadError::Connect {
                message: "x".repeat(MAX_EVENT_STRING * 2),
            },
        };
        match loud.into_net(0) {
            Some(NetEvent::Failed { error, .. }) => match error.downcast_ref() {
                Some(crate::LoadError::Connect { message }) => {
                    assert_eq!(message.len(), MAX_EVENT_STRING)
                }
                other => panic!("not a connect error: {other:?}"),
            },
            other => panic!("not a failure: {other:?}"),
        }
    }

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
