//! The wire vocabulary of the cookie vault: what the broker (and, directly,
//! the network process) may ask of it.
//!
//! Identity is never a claim. On the broker's link the `zone` is the broker's
//! own bookkeeping. On the network process's link every `Get`/`Store` names a
//! ticket the broker granted for that one request, and the vault answers from
//! the grant's scope, not the caller's. `visible_only` is the HttpOnly split -
//! the `document.cookie` view versus the full set that goes on the wire -
//! enforced here rather than in whoever asks.

use crate::engine::cookies::DefaultCookieJar;
use serde::{Deserialize, Serialize};

/// The argv role name the broker re-execs itself with.
pub const VAULT_ROLE: &str = "vault";

/// Correlates a reply with its request on a link. Assigned by the asker; the
/// vault only echoes it back.
pub type Tag = u64;

pub use crate::net::process::protocol::{CookieScope, SameSite, Ticket};

/// Broker (or network process) → vault.
#[derive(Debug, Serialize, Deserialize)]
pub enum ToVault {
    /// Prove the child is a vault before anything is entrusted to it.
    Ping,
    /// Start holding a zone's cookies, seeded from what the broker's store had
    /// persisted for it (`None` starts empty).
    OpenZone {
        zone: String,
        snapshot: Option<DefaultCookieJar>,
    },
    /// The zone closed; its jar is dropped here (the broker persisted the last
    /// snapshot it was sent).
    CloseZone {
        zone: String,
    },
    /// The `Cookie` header value for a request, or `None` when nothing applies.
    Get {
        tag: Tag,
        scope: CookieScope,
        url: String,
        /// Only cookies a script may see (no HttpOnly): the `document.cookie` view.
        visible_only: bool,
    },
    /// Record a response's `Set-Cookie` headers. Answered with
    /// [`FromVault::Stored`] once the jar holds them - the network process
    /// waits for that before it reports the response, so the store is done
    /// before the broker revokes the ticket. The jar's new state follows as a
    /// [`FromVault::Snapshot`] on the broker link.
    Store {
        tag: Tag,
        scope: CookieScope,
        url: String,
        set_cookie: Vec<String>,
    },
    /// Broker only: let the network process act on `scope` under its ticket
    /// for the length of one request. Answered with [`FromVault::Granted`]
    /// before the request is dispatched, so the grant is in place first.
    Grant {
        tag: Tag,
        scope: CookieScope,
    },
    /// Broker only: the request is over.
    Revoke {
        ticket: Ticket,
    },
    /// Every cookie of a zone, `(url, "name=value")`, for the embedder API.
    GetAll {
        tag: Tag,
        zone: String,
    },
    Clear {
        zone: String,
    },
    Remove {
        zone: String,
        url: String,
        name: String,
    },
    RemoveForUrl {
        zone: String,
        url: String,
    },
    PurgeExpired {
        zone: String,
    },
    Shutdown,
}

/// Vault → broker (or network process).
#[derive(Debug, Serialize, Deserialize)]
pub enum FromVault {
    Pong,
    Granted {
        tag: Tag,
    },
    /// The grant was not made; the request goes without cookies.
    Refused {
        tag: Tag,
    },
    Stored {
        tag: Tag,
    },
    Cookies {
        tag: Tag,
        header: Option<String>,
    },
    All {
        tag: Tag,
        cookies: Vec<(String, String)>,
    },
    /// A zone's jar changed: the broker persists this through the zone's
    /// cookie store. The vault itself never touches a file.
    Snapshot {
        zone: String,
        jar: DefaultCookieJar,
    },
}
