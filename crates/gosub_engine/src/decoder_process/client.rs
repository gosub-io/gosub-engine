//! The broker's side of image decoding: one throwaway process per image.

use crate::decoder_process::protocol::{FromDecoder, ToDecoder};
use gosub_interface::media_decoder::{BrokeredDecode, DecodeError, ImageDecoder, RasterImage};
use std::time::{Duration, Instant};

/// The argv role name the broker re-execs itself with.
pub const DECODER_ROLE: &str = "decoder";

/// How long one image may take, start to finish.
const DECODE_TIMEOUT: Duration = Duration::from_secs(10);

/// Largest decoded image accepted, as raw RGBA.
///
/// 256 MiB is about 8000×8000 - beyond any plausible page image, and far below
/// what a decompression bomb would ask for. The cap is enforced while
/// accumulating, so an over-large claim costs nothing to reject.
const MAX_DECODED_BYTES: u64 = 256 * 1024 * 1024;

/// The largest dimension accepted in a header, so an absurd claim is rejected
/// before it is multiplied out.
const MAX_DIMENSION: u32 = 32_768;

/// Decodes each image in its own short-lived, sandboxed process.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProcessImageDecoder;

impl ImageDecoder for ProcessImageDecoder {
    fn decode(&self, mime: Option<&str>, bytes: &[u8]) -> Result<BrokeredDecode, DecodeError> {
        decode_in_child(mime, bytes)
    }
}

fn decode_in_child(mime: Option<&str>, bytes: &[u8]) -> Result<BrokeredDecode, DecodeError> {
    // Same guard as the network process: a child that never dispatched is
    // running embedder startup, and spawning from there would repeat forever.
    if crate::child_process::is_child_process() {
        return Err(DecodeError::Failed(
            "refusing to spawn a decoder from an undispatched child process".into(),
        ));
    }

    let exe = std::env::current_exe().map_err(|e| DecodeError::Failed(e.to_string()))?;
    let (ours, theirs) = gosub_ipc::channel::Channel::pair().map_err(|e| DecodeError::Failed(e.to_string()))?;

    let mut child = gosub_sandbox::spawn::spawn(
        &exe,
        &[crate::child_process::ROLE_FLAG, DECODER_ROLE],
        theirs,
        // Nothing to reach: a decoder has no business on the network, so it goes
        // in an empty namespace as well as being denied the syscalls.
        gosub_sandbox::NamespaceIsolation::Full,
        gosub_sandbox::spawn::ContainerProfile {
            name: "gosub-decoder",
            internet: false,
            fs_grant: None,
        },
    )
    .map_err(|e| DecodeError::Failed(format!("could not spawn a decoder: {e}")))?;

    if let Err(e) = gosub_sandbox::confine_spawned_child(&child) {
        log::warn!("could not apply parent-side confinement to the decoder: {e}");
    }

    let result = exchange(ours, mime, bytes);

    // Kill before reaping, on every path: a decoder that will not exit - wedged
    // or hostile - must not be able to hold this thread open.
    let _ = child.kill();
    let _ = child.wait();

    result
}

/// Send the image and read back what the decoder makes of it.
fn exchange(
    channel: gosub_ipc::channel::Channel,
    mime: Option<&str>,
    bytes: &[u8],
) -> Result<BrokeredDecode, DecodeError> {
    let mut link = gosub_ipc::Endpoint::from_channel(channel).map_err(|e| DecodeError::Failed(e.to_string()))?;

    let deadline = Instant::now() + DECODE_TIMEOUT;
    // The socket timeouts are what make the deadline real: without them a
    // decoder that simply stops talking would block this thread forever.
    let _ = link.tx.set_write_timeout(Some(DECODE_TIMEOUT));
    let _ = link.rx.set_read_timeout(Some(DECODE_TIMEOUT));

    let request = ToDecoder::Decode {
        mime: mime.map(str::to_string),
        bytes: bytes.to_vec(),
    };
    link.send(&request)
        .map_err(|e| DecodeError::Failed(format!("could not hand the image to the decoder: {e}")))?;

    let mut pending: Option<(u32, u32, u64, Vec<u8>)> = None;
    loop {
        if Instant::now() >= deadline {
            return Err(DecodeError::TimedOut);
        }
        let msg = link.recv::<FromDecoder>().map_err(|e| {
            if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut {
                DecodeError::TimedOut
            } else {
                DecodeError::Failed(format!("the decoder stopped responding: {e}"))
            }
        })?;

        match msg {
            FromDecoder::Vector => return Ok(BrokeredDecode::Vector),
            FromDecoder::Failed(why) => return Err(DecodeError::Unsupported(why)),
            FromDecoder::RasterHeader { width, height, len } => {
                if pending.is_some() {
                    return Err(DecodeError::Failed("the decoder sent two headers".into()));
                }
                // Bound the claim before it is multiplied out or used to size
                // anything: every value here came from the untrusted side.
                if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
                    return Err(DecodeError::Failed(format!(
                        "implausible image dimensions {width}x{height}"
                    )));
                }
                let expected = u64::from(width) * u64::from(height) * 4;
                if len != expected {
                    return Err(DecodeError::Failed(format!(
                        "the decoder claimed {len} bytes for a {width}x{height} image, expected {expected}"
                    )));
                }
                if len > MAX_DECODED_BYTES {
                    return Err(DecodeError::Failed(format!(
                        "decoded image of {len} bytes is too large"
                    )));
                }
                pending = Some((width, height, len, Vec::with_capacity(len as usize)));
            }
            FromDecoder::Chunk(chunk) => {
                let Some((_, _, len, buffer)) = pending.as_mut() else {
                    return Err(DecodeError::Failed("the decoder sent pixels before a header".into()));
                };
                // Checked per chunk, so a flood is cut off at the first byte past
                // the agreed size rather than after it has been buffered.
                if buffer.len() as u64 + chunk.len() as u64 > *len {
                    return Err(DecodeError::Failed(
                        "the decoder sent more pixels than it announced".into(),
                    ));
                }
                buffer.extend_from_slice(&chunk);
            }
            FromDecoder::RasterEnd => {
                let Some((width, height, len, buffer)) = pending.take() else {
                    return Err(DecodeError::Failed(
                        "the decoder ended a transfer it never started".into(),
                    ));
                };
                if buffer.len() as u64 != len {
                    return Err(DecodeError::Failed(format!(
                        "the decoder sent {} of {len} announced bytes",
                        buffer.len()
                    )));
                }
                return Ok(BrokeredDecode::Raster(RasterImage {
                    width,
                    height,
                    rgba: bytes::Bytes::from(buffer),
                }));
            }
        }
    }
}
