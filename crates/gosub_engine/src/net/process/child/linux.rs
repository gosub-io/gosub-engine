//! Linux: a streamed body crosses to the broker through a shared-memory ring
//! whose fd is passed on the link. The API here is what `super` calls;
//! `portable.rs` is its stand-in.

use crate::net::process::protocol::{FetchOutcome, FromNet, RequestTag};
use gosub_ipc::EndpointTx;
use gosub_sonar::net::shared_body::SharedBody;
use parking_lot::Mutex;
use std::sync::Arc;

/// Bodies may stream: the link carries the ring's fd.
pub(super) const STREAMING: bool = true;

/// Ring window for streamed bodies. Small on purpose: a large body wraps
/// through it many times, and neither side ever holds more than this (plus one
/// chunk) for the transport.
const RING_CAPACITY: u32 = 256 * 1024;

/// How many chunks the pump may fall behind the fetcher before the body drops
/// it as a slow subscriber. The ring's backpressure stalls the pump whenever
/// the broker is not draining; this queue absorbs that stall (16 KiB chunks:
/// at most 16 MiB held) so backpressure costs memory here, never bytes.
const PUMP_QUEUE: usize = 1024;

type BodyChunks = futures_util::stream::BoxStream<'static, Result<bytes::Bytes, gosub_sonar::net::types::NetError>>;

/// A response head whose body is still arriving: sent first, then the ring's
/// fd, then the body pumped into the ring for as long as it lasts.
pub(super) struct Streamed {
    head: FetchOutcome,
    ring: std::os::fd::OwnedFd,
    producer: gosub_ipc::ring::RingProducer,
    body: BodyPump,
}

/// The fetcher's side of a streamed body, as the pump needs it.
struct BodyPump {
    /// Subscribed the moment the response arrived: the body has no replay,
    /// so a later subscription would miss its first chunks.
    chunks: BodyChunks,
    /// What `Content-Length` promises past the peek, when it says.
    expected: Option<u64>,
}

/// Subscribe to the body - first thing, before anything can yield: the
/// fetcher is already pushing chunks, and nobody replays them - and set up
/// the ring it will be pumped into.
pub(super) fn begin_stream(
    head: FetchOutcome,
    expected: Option<u64>,
    shared: Arc<SharedBody>,
) -> Result<Streamed, String> {
    let chunks = shared.subscribe_with_cap(PUMP_QUEUE);
    let (producer, ring) = gosub_ipc::ring::RingProducer::create(RING_CAPACITY).map_err(|e| e.to_string())?;
    Ok(Streamed {
        head,
        ring,
        producer,
        body: BodyPump { chunks, expected },
    })
}

impl Streamed {
    /// Head and ring fd back to back, under one lock, so nothing else on the
    /// link comes between them; then the body. A write error means the broker
    /// went away, which the recv loop notices too.
    pub(super) async fn deliver(self, tag: RequestTag, link_tx: &Arc<Mutex<EndpointTx>>) {
        use std::os::fd::AsRawFd as _;
        {
            let mut tx = link_tx.lock();
            if tx
                .send(&FromNet::Reply {
                    tag,
                    outcome: self.head,
                })
                .is_err()
                || tx.send_fd(self.ring.as_raw_fd()).is_err()
            {
                return;
            }
        }
        drop(self.ring); // the broker holds its duplicate; the mapping keeps ours
        pump(self.producer, self.body).await;
    }
}

/// Move a body from the fetcher into the ring as it arrives. The producer's
/// `write_all` blocks (bounded) when the ring is full - the backpressure that
/// keeps this process from buffering - so it runs off the async worker. An
/// error from the fetcher, or a consumer that stopped draining, abandons the
/// stream: the producer drops unfinished and the consumer sees an abort.
///
/// A subscriber the body dropped for falling behind gets an error (sonar
/// 0.4), which ends the pump without `finish`; the byte count against
/// `Content-Length` is the second check: a truncated body must abort, never
/// look complete.
async fn pump(mut producer: gosub_ipc::ring::RingProducer, body: BodyPump) {
    use futures_util::StreamExt as _;
    let BodyPump { mut chunks, expected } = body;
    let mut pumped = 0u64;
    while let Some(chunk) = chunks.next().await {
        let Ok(chunk) = chunk else {
            return;
        };
        pumped += chunk.len() as u64;
        if tokio::task::block_in_place(|| producer.write_all(&chunk)).is_err() {
            return;
        }
    }
    if expected.is_some_and(|expected| pumped != expected) {
        eprintln!("[net] body stream fell behind the fetcher ({pumped} bytes pumped); aborting it");
        return;
    }
    producer.finish();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain(ring: std::os::fd::OwnedFd) -> std::io::Result<Vec<u8>> {
        let mut consumer = gosub_ipc::ring::RingConsumer::open(ring)?;
        let mut out = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            match consumer.read(&mut buf)? {
                0 => return Ok(out),
                n => out.extend_from_slice(&buf[..n]),
            }
        }
    }

    fn body(shared: &Arc<SharedBody>, cap: usize, expected: Option<u64>) -> BodyPump {
        BodyPump {
            chunks: shared.subscribe_with_cap(cap),
            expected,
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_complete_body_finishes_the_ring() {
        let shared = Arc::new(SharedBody::new(8));
        let pump_body = body(&shared, 8, Some(6));
        let (producer, ring) = gosub_ipc::ring::RingProducer::create(1 << 16).unwrap();
        let drained = std::thread::spawn(move || drain(ring));
        shared.push(bytes::Bytes::from_static(b"abc"));
        shared.push(bytes::Bytes::from_static(b"def"));
        shared.finish();
        pump(producer, pump_body).await;
        assert_eq!(drained.join().unwrap().unwrap(), b"abcdef");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_body_short_of_its_content_length_aborts_the_ring() {
        let shared = Arc::new(SharedBody::new(1));
        // A one-slot queue nobody reads yet: the second push drops the subscriber.
        let pump_body = body(&shared, 1, Some(6));
        let (producer, ring) = gosub_ipc::ring::RingProducer::create(1 << 16).unwrap();
        let drained = std::thread::spawn(move || drain(ring));
        shared.push(bytes::Bytes::from_static(b"abc"));
        shared.push(bytes::Bytes::from_static(b"def"));
        shared.finish();
        pump(producer, pump_body).await;
        assert!(
            drained.join().unwrap().is_err(),
            "a truncated body must not read as EOF"
        );
    }

    /// A chunked body (no Content-Length) that finishes right after dropping
    /// the pump: only sonar's lag error tells this from a complete body.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_body_that_finishes_after_dropping_the_pump_aborts_the_ring() {
        let shared = Arc::new(SharedBody::new(1));
        let pump_body = body(&shared, 1, None);
        let (producer, ring) = gosub_ipc::ring::RingProducer::create(1 << 16).unwrap();
        let drained = std::thread::spawn(move || drain(ring));
        shared.push(bytes::Bytes::from_static(b"abc"));
        shared.push(bytes::Bytes::from_static(b"def"));
        shared.finish();
        pump(producer, pump_body).await;
        assert!(
            drained.join().unwrap().is_err(),
            "a body finished after the drop must not read as EOF"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_dropped_subscriber_of_a_live_body_aborts_the_ring() {
        let shared = Arc::new(SharedBody::new(1));
        let pump_body = body(&shared, 1, None);
        let (producer, ring) = gosub_ipc::ring::RingProducer::create(1 << 16).unwrap();
        let drained = std::thread::spawn(move || drain(ring));
        shared.push(bytes::Bytes::from_static(b"abc"));
        shared.push(bytes::Bytes::from_static(b"def"));
        // Not finished: the body is still streaming when the pump's stream ends.
        pump(producer, pump_body).await;
        assert!(
            drained.join().unwrap().is_err(),
            "a dropped subscriber must not read as EOF"
        );
    }
}
