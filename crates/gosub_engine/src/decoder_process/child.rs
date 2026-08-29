//! The image decoder process: one image, then gone.

use crate::decoder_process::protocol::{FromDecoder, ToDecoder, CHUNK_BYTES};
use gosub_ipc::Endpoint;
use gosub_render_pipeline::common::media::{
    render_svg_tree_to_image, DecodedMedia, MediaDecoderRegistry, RasterDecoder, SvgDecoder,
};

/// An SVG is rasterized at its intrinsic size, scaled down to fit this.
const MAX_SVG_SIDE: f32 = 4096.0;

/// Decode one image for the broker, then return the process exit code.
pub fn serve(mut link: Endpoint) -> i32 {
    // Renamed before the lockdown takes /proc away; there is no per-image
    // identity to show - this process decodes exactly one and exits.
    gosub_sandbox::capture_process_title_region();
    gosub_sandbox::set_process_title("gosub-decoder", "gosub: image decoder");

    // The default set, minus system fonts for SVG: discovering and lazily
    // loading them walks the filesystem, which the lockdown below forbids.
    // SVG text therefore renders without fonts here; the alternative is
    // parsing untrusted SVG in the caller. Nothing here opens a file, a
    // socket or a program, so the tightest profile applies, installed before
    // the untrusted bytes are so much as read.
    let mut registry = MediaDecoderRegistry::new();
    registry.register(Box::new(SvgDecoder::without_system_fonts()));
    registry.register(Box::new(RasterDecoder));

    gosub_sandbox::lock_down_decoder();

    let (mime, bytes) = match link.recv::<ToDecoder>() {
        Ok(ToDecoder::Decode { mime, bytes }) => (mime, bytes),
        Ok(ToDecoder::Dimensions { mime, bytes }) => {
            let reply = match registry.dimensions(mime.as_deref(), &bytes) {
                Some((width, height)) => FromDecoder::Dimensions { width, height },
                None => FromDecoder::Failed("not an image".into()),
            };
            return if link.send(&reply).is_ok() { 0 } else { 1 };
        }
        // The broker went away before asking for anything; nothing to do.
        Err(_) => return 0,
    };
    let reply = match registry.decode(mime.as_deref(), &bytes) {
        // Already normalised to RGBA8 by the decoder, so this is the layout the
        // broker expects and no conversion is needed here.
        Ok(DecodedMedia::Raster(image)) => {
            return send_raster(&mut link, image.width(), image.height(), image.as_raw());
        }
        // A tree cannot cross the boundary: pixels at the intrinsic size do.
        Ok(DecodedMedia::Vector(tree)) => {
            let size = tree.size();
            let (w, h) = (size.width().max(1.0), size.height().max(1.0));
            let scale = (MAX_SVG_SIDE / w).min(MAX_SVG_SIDE / h).min(1.0);
            let (w, h) = ((w * scale).round().max(1.0) as u32, (h * scale).round().max(1.0) as u32);
            match render_svg_tree_to_image(&tree, w, h) {
                Some(image) => return send_raster(&mut link, image.width(), image.height(), image.as_raw()),
                None => FromDecoder::Failed("could not rasterize the SVG".into()),
            }
        }
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
