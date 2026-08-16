//! The image decoder process: one image, then gone.

use crate::decoder_process::protocol::{FromDecoder, ToDecoder, CHUNK_BYTES};
use gosub_ipc::Endpoint;
use gosub_render_pipeline::common::media::{DecodedMedia, MediaDecoderRegistry};

/// Decode one image for the broker, then return the process exit code.
pub fn serve(mut link: Endpoint) -> i32 {
    // Nothing below opens a file, a socket or a program, so the tightest profile
    // applies. Installed before the untrusted bytes are so much as read.
    gosub_sandbox::lock_down_renderer();

    let ToDecoder::Decode { mime, bytes } = match link.recv::<ToDecoder>() {
        Ok(msg) => msg,
        // The broker went away before asking for anything; nothing to do.
        Err(_) => return 0,
    };

    let registry = MediaDecoderRegistry::with_defaults();
    let reply = match registry.decode(mime.as_deref(), &bytes) {
        // Already normalised to RGBA8 by the decoder, so this is the layout the
        // broker expects and no conversion is needed here.
        Ok(DecodedMedia::Raster(image)) => {
            return send_raster(&mut link, image.width(), image.height(), image.as_raw());
        }
        Ok(DecodedMedia::Vector(_)) => FromDecoder::Vector,
        Err(e) => FromDecoder::Failed(e.to_string()),
    };

    if link.send(&reply).is_err() {
        return 1;
    }
    0
}

/// Send dimensions, then the pixels in bounded pieces, then the terminator.
fn send_raster(link: &mut Endpoint, width: u32, height: u32, rgba: &[u8]) -> i32 {
    let header = FromDecoder::RasterHeader {
        width,
        height,
        len: rgba.len() as u64,
    };
    if link.send(&header).is_err() {
        return 1;
    }
    for chunk in rgba.chunks(CHUNK_BYTES) {
        if link.send(&FromDecoder::Chunk(chunk.to_vec())).is_err() {
            return 1;
        }
    }
    if link.send(&FromDecoder::RasterEnd).is_err() {
        return 1;
    }
    0
}
