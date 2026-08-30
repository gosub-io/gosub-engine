//! Broker ↔ storage service messages. The area on a request is the broker's
//! word, never a renderer's.

use serde::{Deserialize, Serialize};

/// The argv role name the broker re-execs itself with.
pub const STORAGE_ROLE: &str = "storage";

pub type Tag = u64;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct AreaKey {
    pub zone: String,
    pub partition: String,
    pub origin: String,
}

/// Broker → storage service.
#[derive(Debug, Serialize, Deserialize)]
pub enum ToStorage {
    Ping,
    Get {
        tag: Tag,
        area: AreaKey,
        key: String,
    },
    Set {
        tag: Tag,
        area: AreaKey,
        key: String,
        value: String,
    },
    Remove {
        tag: Tag,
        area: AreaKey,
        key: String,
    },
    Clear {
        tag: Tag,
        area: AreaKey,
    },
    Keys {
        tag: Tag,
        area: AreaKey,
    },
    Len {
        tag: Tag,
        area: AreaKey,
    },
    /// Run the escape audit in the service and report it.
    Audit {
        tag: Tag,
    },
    Shutdown,
}

/// Storage service → broker.
#[derive(Debug, Serialize, Deserialize)]
pub enum FromStorage {
    Pong,
    Value {
        tag: Tag,
        value: Option<String>,
    },
    /// `error` names a refused write (quota, size).
    Done {
        tag: Tag,
        error: Option<String>,
    },
    Keys {
        tag: Tag,
        keys: Vec<String>,
    },
    Len {
        tag: Tag,
        len: u64,
    },
    Audit {
        tag: Tag,
        report: gosub_sandbox::audit::AuditReport,
    },
}
