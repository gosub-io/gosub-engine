//! The broker↔decoder wire vocabulary.
//!
//! ## Why pixels arrive in pieces
//!
//! A decoded image is far larger than its encoded form: a 4000×3000 photo is
//! 48 MiB of RGBA, well past the 16 MiB cap that keeps a corrupt length prefix
//! from forcing a huge allocation. Rather than raise that cap for every link in
//! the engine, the pixels are sent as bounded chunks and the receiver accumulates
//! them against its own total limit. The zero-copy alternative — a sealed
//! shared-memory buffer, `gosub_ipc::shm` — is already imported and is the
//! natural upgrade; chunking keeps this portable in the meantime.

use serde::{Deserialize, Serialize};

/// Largest chunk of pixel data in one frame, comfortably inside the transport's
/// frame cap with room for the enum's own encoding.
pub const CHUNK_BYTES: usize = 4 * 1024 * 1024;

/// Broker → decoder. Exactly one of these is ever sent: the process handles a
/// single image and exits.
#[derive(Debug, Serialize, Deserialize)]
pub enum ToDecoder {
    Decode {
        /// The `Content-Type` the response claimed, if any. A hint only — the
        /// decoder sniffs the bytes, since this comes from the network.
        mime: Option<String>,
        bytes: Vec<u8>,
    },
}

/// Decoder → broker.
#[derive(Debug, Serialize, Deserialize)]
pub enum FromDecoder {
    /// Dimensions first, so the receiver can size and bound the transfer before
    /// any pixels arrive. Both are claims to be checked, not facts.
    RasterHeader {
        width: u32,
        height: u32,
        len: u64,
    },
    /// A piece of the RGBA buffer, in order.
    Chunk(Vec<u8>),
    /// All chunks sent.
    RasterEnd,
    /// The bytes are a vector format. No payload: a parsed tree cannot cross the
    /// boundary, so the broker parses it (see `gosub_interface::media_decoder`).
    Vector,
    Failed(String),
}
