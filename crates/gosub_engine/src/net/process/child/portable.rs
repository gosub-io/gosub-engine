//! Off Linux: no fd passing, so no streamed bodies and no vault line. The
//! broker buffers bodies and attaches cookies itself; this is `linux.rs`'s
//! API declining.

use crate::net::process::protocol::{CookieScope, FetchOutcome, RequestTag};
use gosub_ipc::{Endpoint, EndpointRx, EndpointTx};
use gosub_sonar::net::shared_body::SharedBody;
use gosub_sonar::net::types::FetchResultMeta;
use parking_lot::Mutex;
use std::sync::Arc;

/// Bodies never stream: the broker asks for them buffered.
pub(super) const STREAMING: bool = false;

pub(super) fn escape_audit() -> Option<gosub_sandbox::audit::AuditReport> {
    None
}

/// Never constructed here; the type exists so the caller has no branch.
pub(super) enum Streamed {}

pub(super) fn begin_stream(
    _head: FetchOutcome,
    _expected: Option<u64>,
    _shared: Arc<SharedBody>,
) -> Result<Streamed, String> {
    Err("body streaming needs fd passing, which only Linux has".into())
}

impl Streamed {
    pub(super) async fn deliver(self, _tag: RequestTag, _link_tx: &Arc<Mutex<EndpointTx>>) {
        match self {}
    }
}

/// A line that is never used: cookies come from the broker on these platforms.
pub(super) struct VaultLink;

impl VaultLink {
    pub(super) fn new(_link: Endpoint) -> Self {
        Self
    }
}

pub(super) fn adopt_vault_line(_rx: &mut EndpointRx) -> Result<VaultLink, String> {
    Err("a vault line is a Linux thing".into())
}

pub(super) fn vault_cookies(_vault: &Mutex<Option<VaultLink>>, _scope: &CookieScope, _url: &str) -> Option<String> {
    None
}

pub(super) fn vault_store(_vault: &Mutex<Option<VaultLink>>, _scope: &CookieScope, _meta: &FetchResultMeta) {}
