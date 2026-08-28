//! The wire vocabulary of the cookie vault: what the broker (and, directly,
//! the network process) may ask of it.
//!
//! Identity is never a claim: the `zone` on every message is stamped by the
//! broker from its own bookkeeping, and the vault answers for that partition
//! only. `visible_only` is the HttpOnly split - the `document.cookie` view
//! versus the full set that goes on the wire - enforced here rather than in
//! whoever asks.

use crate::engine::cookies::DefaultCookieJar;
use serde::{Deserialize, Serialize};

/// The argv role name the broker re-execs itself with.
pub const VAULT_ROLE: &str = "vault";

/// Correlates a reply with its request on a link. Assigned by the asker; the
/// vault only echoes it back.
pub type Tag = u64;

pub use crate::net::process::protocol::{CookieScope, SameSite};

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
    /// Record a response's `Set-Cookie` headers. Fire-and-forget; the jar's
    /// new state follows as a [`FromVault::Snapshot`] on the broker link.
    Store {
        zone: String,
        url: String,
        top_level: Option<String>,
        set_cookie: Vec<String>,
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
