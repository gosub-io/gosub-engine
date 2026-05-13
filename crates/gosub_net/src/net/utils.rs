use bytes::Bytes;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::pin::Pin;
use tokio::io::ReadBuf;
use url::Url;

/// Normalizes a URL by removing its fragment and returning it as a string.
pub fn normalize_url(u: &Url) -> String {
    let mut u = u.clone();
    u.set_fragment(None);
    u.as_str().to_string()
}

/// Computes a short hash for a given byte slice.
pub fn short_hash(bytes: &[u8]) -> u64 {
    let mut h = DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

/// Returns a URL string truncated to `max` characters with `...` suffix.
pub fn short_url(u: &Url, max: usize) -> String {
    let s = u.as_str();
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

/// Minimal async reader backed by an in-memory `Bytes` buffer.
pub struct BytesAsyncReader {
    pub data: Bytes,
    pub pos: usize,
}

impl tokio::io::AsyncRead for BytesAsyncReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let remaining = self.data.len().saturating_sub(self.pos);
        if remaining == 0 {
            return std::task::Poll::Ready(Ok(()));
        }
        let to_copy = std::cmp::min(remaining, buf.remaining());
        let end = self.pos + to_copy;
        buf.put_slice(&self.data[self.pos..end]);
        self.pos = end;
        std::task::Poll::Ready(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    #[test]
    fn normalize_url_strips_fragment() {
        let u = Url::parse("https://example.org/a/b#frag").unwrap();
        assert_eq!(normalize_url(&u), "https://example.org/a/b");
    }

    #[test]
    fn short_hash_differs_for_diff_inputs() {
        assert_ne!(short_hash(b"abc"), short_hash(b"abd"));
    }

    #[test]
    fn short_url_truncates() {
        let u = Url::parse("https://example.org/very/long/path/here").unwrap();
        let s = short_url(&u, 16);
        assert!(s.ends_with("..."));
        assert!(s.len() <= 19);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn bytes_async_reader_reads_all() {
        let data = Bytes::from_static(b"hello world");
        let mut r = BytesAsyncReader { data, pos: 0 };
        let mut out = Vec::new();
        r.read_to_end(&mut out).await.unwrap();
        assert_eq!(&out[..], b"hello world");
        let n = r.read(&mut [0u8; 8]).await.unwrap();
        assert_eq!(n, 0);
    }
}
