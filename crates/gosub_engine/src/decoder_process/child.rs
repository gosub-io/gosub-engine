//! The image decoder process: one image, then gone.

use crate::decoder_process::protocol::{FromDecoder, ToDecoder, CHUNK_BYTES};
use gosub_ipc::Endpoint;
use gosub_render_pipeline::common::media::{DecodedMedia, MediaDecoderRegistry, RasterDecoder, SvgDecoder};

/// Decode one image for the broker, then return the process exit code.
pub fn serve(mut link: Endpoint) -> i32 {
    // Renamed before the lockdown takes /proc away; there is no per-image
    // identity to show - this process decodes exactly one and exits.
    gosub_sandbox::capture_process_title_region();
    gosub_sandbox::set_process_title("gosub-decoder", "gosub: image decoder");

    // The default set, minus system fonts for SVG: discovering and lazily
    // loading them walks the filesystem, which the lockdown below forbids -
    // and a font-less parse is all this process owes the broker (see
    // `FromDecoder::Vector`). Nothing here opens a file, a socket or a
    // program, so the tightest profile applies, installed before the
    // untrusted bytes are so much as read.
    let mut registry = MediaDecoderRegistry::new();
    registry.register(Box::new(SvgDecoder::without_system_fonts()));
    registry.register(Box::new(RasterDecoder));

    gosub_sandbox::lock_down_decoder();

    let ToDecoder::Decode { mime, bytes } = match link.recv::<ToDecoder>() {
        Ok(msg) => msg,
        // The broker went away before asking for anything; nothing to do.
        Err(_) => return 0,
    };
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
