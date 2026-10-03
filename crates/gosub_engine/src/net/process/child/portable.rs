//! Off Linux: no fd passing, so no streamed bodies. The broker buffers
//! bodies; this is `linux.rs`'s API declining.

use crate::net::process::protocol::{FetchOutcome, RequestTag};
use gosub_ipc::EndpointTx;
use gosub_sonar::net::shared_body::SharedBody;
use parking_lot::Mutex;
use std::sync::Arc;

/// Bodies never stream: the broker asks for them buffered.
pub(super) const STREAMING: bool = false;

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
