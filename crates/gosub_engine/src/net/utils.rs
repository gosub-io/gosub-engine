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
    // Lossless: a reader that must have the whole body is waited for, never dropped.
    let mut reader = SharedBody::combined_reader_lossless(peek_buf, shared);
    if let Err(e) = reader.read_to_end(&mut out).await {
        return Err(NetError::Io(Arc::new(e)).into());
    }
    Ok(Bytes::from(out))
}

/// Like [`stream_to_bytes`], but keeps only the first `max_bytes`, and says whether the body
/// was longer. The rest is not read: the reader is dropped there, and a body stops waiting for
/// a dropped lossless reader.
pub async fn stream_to_bytes_capped(
    peek_buf: PeekBuf,
    shared: Arc<SharedBody>,
    max_bytes: usize,
) -> anyhow::Result<(Bytes, bool)> {
    let mut out = Vec::new();
    let reader = SharedBody::combined_reader_lossless(peek_buf, shared);
    // One byte past the cap tells a body that was cut from one that fits exactly.
    let limit = (max_bytes as u64).saturating_add(1);
    if let Err(e) = reader.take(limit).read_to_end(&mut out).await {
        return Err(NetError::Io(Arc::new(e)).into());
    }
    let cut = out.len() > max_bytes;
    out.truncate(max_bytes);
    Ok((Bytes::from(out), cut))
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

    #[tokio::test(flavor = "current_thread")]
    async fn a_capped_read_keeps_the_first_bytes() {
        let shared = Arc::new(SharedBody::new(8));
        let writer = shared.clone();
        let producer = tokio::spawn(async move {
            writer.push_wait(Bytes::from_static(b"-BODY-1")).await;
            writer.push_wait(Bytes::from_static(b"-BODY-2")).await;
            writer.finish();
        });

        let (body, cut) = stream_to_bytes_capped(PeekBuf::from_slice(b"HEAD"), shared, 9)
            .await
            .unwrap();
        assert!(cut);
        assert_eq!(&body[..], b"HEAD-BODY");
        producer.await.unwrap();
    }

    /// A body that never ends (or is huge) does not hold the read up past the cap.
    #[tokio::test(flavor = "current_thread")]
    async fn a_capped_read_does_not_wait_for_the_end_of_the_body() {
        let shared = Arc::new(SharedBody::new(8));
        shared.push(Bytes::from_static(b"-BODY-1"));

        let read = stream_to_bytes_capped(PeekBuf::from_slice(b"HEAD"), shared.clone(), 6);
        let body = tokio::time::timeout(std::time::Duration::from_secs(5), read)
            .await
            .expect("the read waited for the rest of the body")
            .unwrap()
            .0;
        assert_eq!(&body[..], b"HEAD-B");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_capped_read_under_the_cap_is_whole() {
        let shared = Arc::new(SharedBody::new(8));
        shared.push(Bytes::from_static(b"-BODY"));
        shared.finish();

        let (body, cut) = stream_to_bytes_capped(PeekBuf::from_slice(b"HEAD"), shared, 64)
            .await
            .unwrap();
        assert!(!cut);
        assert_eq!(&body[..], b"HEAD-BODY");
    }
}
