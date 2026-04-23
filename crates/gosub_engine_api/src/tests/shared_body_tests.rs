use bytes::{Bytes, BytesMut};
use futures_util::StreamExt;
use tokio::io::AsyncReadExt;

use gosub_engine_api::net::shared_body::SharedBody;
// adjust path to your crate
use gosub_engine_api::net::types::{FetchResult, FetchResultMeta, NetError};

#[tokio::test(flavor = "current_thread")]
async fn shared_body_broadcasts_and_finishes() {
    let sb = SharedBody::new(8);

    let mut s1 = sb.subscribe_stream();
    let mut s2 = sb.subscribe_stream();

    sb.push(Bytes::from_static(b"hello"));
    sb.push(Bytes::from_static(b" world"));
    sb.finish();

    let a1 = s1.next().await.unwrap().unwrap();
    let a2 = s1.next().await.unwrap().unwrap();
    assert_eq!(&a1[..], b"hello");
    assert_eq!(&a2[..], b" world");
    assert!(s1.next().await.is_none()); // EOF

    let b1 = s2.next().await.unwrap().unwrap();
    let b2 = s2.next().await.unwrap().unwrap();
    assert_eq!(&b1[..], b"hello");
    assert_eq!(&b2[..], b" world");
    assert!(s2.next().await.is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn shared_body_drops_slow_subscriber() {
    // queue size 1 => easy to overflow
    let sb = SharedBody::new(1);

    let mut slow = sb.subscribe_stream();
    let mut fast = sb.subscribe_stream();

    // Fill, then overflow -> slow will be dropped with lag error
    sb.push(Bytes::from_static(b"A"));
    sb.push(Bytes::from_static(b"B")); // this should drop 'slow'
    sb.push(Bytes::from_static(b"C"));

    // slow consumes first chunk, then should receive an error or end
    let first = slow.next().await.unwrap();
    assert!(first.is_ok());
    let tail = slow.next().await;
    assert!(tail.is_none() || tail.unwrap().is_err(), "slow sub should be dropped");

    // fast should get all chunks after subscribe
    let fa = fast.next().await.unwrap().unwrap();
    let fb = fast.next().await.unwrap().unwrap();
    let fc = fast.next().await.unwrap().unwrap();
    assert_eq!((&fa[..], &fb[..], &fc[..]), (b"A", b"B", b"C"));
}

#[tokio::test(flavor = "current_thread")]
async fn combined_reader_orders_peek_then_tail() {
    let sb = SharedBody::new(8);
    // tail comes in later
    let sb2 = sb.clone();

    let peek = b"PEEK-".to_vec();
    let mut reader = sb.combined_reader(peek.clone());

    // pump some tail
    sb2.push(Bytes::from_static(b"TAIL1"));
    sb2.push(Bytes::from_static(b"TAIL2"));
    sb2.finish();

    let mut buf = Vec::new();
    reader.read_to_end(&mut buf).await.unwrap();
    assert_eq!(&buf[..], b"PEEK-TAIL1TAIL2");
}

#[tokio::test(flavor = "current_thread")]
async fn downconvert_stream_to_buffered() {
    // Simulate Result::Stream -> buffer it
    let sb = Arc::new(SharedBody::new(8));
    let meta = FetchResultMeta {
        final_url: "https://example.test/".parse().unwrap(),
        status: 200,
        status_text: "OK".into(),
        headers: Default::default(),
        content_length: None,
        peek_buf: PeekBuf::empty(),
        has_body: true,
    };

    let peek = b"HEAD".to_vec();

    // Tail in the background
    let sbp = sb.clone();
    tokio::spawn(async move {
        sbp.push(Bytes::from_static(b"-BODY-1"));
        sbp.push(Bytes::from_static(b"-BODY-2"));
        sbp.finish();
    });

    // Use your helper (adjust path) to convert stream -> buffered
    let body = stream_to_bytes(peek, sb.clone()).await;
    match res {
        Ok(body) => {
            assert_eq(&body[..], b"HEAD-BODY-1-BODY-2");
        }
        _ => panic!("expected buffered"),
    }
}
