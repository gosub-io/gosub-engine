//! Small helpers on top of the gosub-sonar streaming types.
//!
//! `gosub-sonar` keeps its internal `utils` module private, so the engine carries its own
//! copy of the pieces it needs.

use crate::engine::types::PeekBuf;
use crate::net::shared_body::SharedBody;
use crate::net::types::NetError;
use bytes::Bytes;
use std::sync::Arc;
use tokio::io::AsyncReadExt;

/// Convert a streaming body to a buffered fetch-result by reading it to the end.
pub async fn stream_to_bytes(peek_buf: PeekBuf, shared: Arc<SharedBody>) -> anyhow::Result<Bytes> {
    let mut out = Vec::with_capacity(peek_buf.len() + 8192);
    let mut reader = SharedBody::combined_reader(peek_buf, shared);
    if let Err(e) = reader.read_to_end(&mut out).await {
        return Err(NetError::Io(Arc::new(e)).into());
    }
    Ok(Bytes::from(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn stream_to_bytes_prepends_peek_buffer() {
        let shared = Arc::new(SharedBody::new(8));
        let writer = shared.clone();
        tokio::spawn(async move {
            writer.push(Bytes::from_static(b"-BODY-1"));
            writer.push(Bytes::from_static(b"-BODY-2"));
            writer.finish();
        });

        let body = stream_to_bytes(PeekBuf::from_slice(b"HEAD"), shared).await.unwrap();
        assert_eq!(&body[..], b"HEAD-BODY-1-BODY-2");
    }
}
