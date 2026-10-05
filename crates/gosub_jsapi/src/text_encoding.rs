//! `TextEncoder`/`TextDecoder` as described by <https://encoding.spec.whatwg.org/#interface-textdecoder>,
//! on top of `encoding_rs` (which implements the Encoding Standard's label
//! table and decoders).

use cow_utils::CowUtils;
use encoding_rs::{CoderResult, Decoder, DecoderResult, Encoding, REPLACEMENT};
use std::error::Error;
use std::fmt;

/// `Display` is prefixed with the JS error class the spec prescribes
/// (`RangeError:` / `TypeError:`) so a binding layer can rethrow the right kind
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncodingError {
    /// The label is unknown or names the replacement encoding (spec: RangeError)
    UnknownLabel(String),
    /// A malformed byte sequence in fatal mode (spec: TypeError)
    Malformed,
}

impl fmt::Display for EncodingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownLabel(label) => {
                write!(f, "RangeError: '{label}' is not a valid encoding label")
            }
            Self::Malformed => f.write_str("TypeError: malformed byte sequence in input"),
        }
    }
}

impl Error for EncodingError {}

/// `TextEncoder`: encodes a string to UTF-8 bytes. Per spec this only ever
/// encodes UTF-8 (lone surrogates must be replaced with U+FFFD *before* the
/// input reaches this type — Rust strings can't carry them anyway).
#[derive(Debug, Default, Clone, Copy)]
pub struct TextEncoder;

impl TextEncoder {
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// The `encoding` attribute: always "utf-8"
    #[must_use]
    pub fn encoding(&self) -> &'static str {
        "utf-8"
    }

    #[must_use]
    pub fn encode(&self, input: &str) -> Vec<u8> {
        input.as_bytes().to_vec()
    }
}

/// `TextDecoder`: a stateful streaming decoder for one of the Encoding
/// Standard's encodings.
pub struct TextDecoder {
    encoding: &'static Encoding,
    decoder: Decoder,
    fatal: bool,
    ignore_bom: bool,
    /// Spec's "do not flush" flag: true while a `stream: true` sequence is open
    do_not_flush: bool,
}

impl TextDecoder {
    /// The `new TextDecoder(label, {fatal, ignoreBOM})` constructor. Label
    /// matching is whitespace-trimmed and case-insensitive per the spec's
    /// label table; unknown labels and the replacement encoding fail.
    pub fn new(label: &str, fatal: bool, ignore_bom: bool) -> Result<Self, EncodingError> {
        let Some(encoding) = Encoding::for_label(label.as_bytes()) else {
            return Err(EncodingError::UnknownLabel(label.to_owned()));
        };
        if encoding == REPLACEMENT {
            return Err(EncodingError::UnknownLabel(label.to_owned()));
        }

        Ok(Self {
            encoding,
            decoder: Self::make_decoder(encoding, ignore_bom),
            fatal,
            ignore_bom,
            do_not_flush: false,
        })
    }

    fn make_decoder(encoding: &'static Encoding, ignore_bom: bool) -> Decoder {
        if ignore_bom {
            encoding.new_decoder_without_bom_handling()
        } else {
            // Removes a leading BOM matching this encoding; never encoding-sniffs
            encoding.new_decoder_with_bom_removal()
        }
    }

    /// The `encoding` attribute: the encoding's canonical name, lowercased
    #[must_use]
    pub fn encoding(&self) -> String {
        self.encoding.name().cow_to_ascii_lowercase().into_owned()
    }

    #[must_use]
    pub fn fatal(&self) -> bool {
        self.fatal
    }

    #[must_use]
    pub fn ignore_bom(&self) -> bool {
        self.ignore_bom
    }

    /// `decode(input, {stream})`. With `stream: true` the decoder keeps
    /// incomplete byte sequences pending for the next call; without it the
    /// input is flushed and the decoder resets for the next decode.
    pub fn decode(&mut self, input: &[u8], stream: bool) -> Result<String, EncodingError> {
        if !self.do_not_flush {
            self.decoder = Self::make_decoder(self.encoding, self.ignore_bom);
        }
        self.do_not_flush = stream;
        let last = !stream;

        // An empty push in stream mode is a spec-level no-op; skipping it also
        // avoids https://github.com/hsivonen/encoding_rs/issues/126 (empty
        // mid-stream pushes corrupt a pending big5/shift_jis/euc-kr lead byte).
        if stream && input.is_empty() {
            return Ok(String::new());
        }

        let mut out = String::new();
        let mut read_total = 0;
        // The encoding_rs decoders write into the String's spare capacity;
        // grow it in doubling chunks until the input is consumed.
        let mut chunk = input.len().max(16);
        loop {
            out.reserve(chunk);
            if self.fatal {
                let (result, read) =
                    self.decoder
                        .decode_to_string_without_replacement(&input[read_total..], &mut out, last);
                read_total += read;
                match result {
                    DecoderResult::InputEmpty => return Ok(out),
                    DecoderResult::Malformed(_, _) => return Err(EncodingError::Malformed),
                    DecoderResult::OutputFull => chunk = chunk.saturating_mul(2),
                }
            } else {
                let (result, read, _had_errors) = self.decoder.decode_to_string(&input[read_total..], &mut out, last);
                read_total += read;
                match result {
                    CoderResult::InputEmpty => return Ok(out),
                    CoderResult::OutputFull => chunk = chunk.saturating_mul(2),
                }
            }
        }
    }
}

impl fmt::Debug for TextDecoder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextDecoder")
            .field("encoding", &self.encoding.name())
            .field("fatal", &self.fatal)
            .field("ignore_bom", &self.ignore_bom)
            .field("do_not_flush", &self.do_not_flush)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoder_basics() {
        let e = TextEncoder::new();
        assert_eq!(e.encoding(), "utf-8");
        assert_eq!(e.encode(""), Vec::<u8>::new());
        assert_eq!(e.encode("a€"), vec![0x61, 0xE2, 0x82, 0xAC]);
    }

    #[test]
    fn decoder_utf8_basics() {
        let mut d = TextDecoder::new("utf-8", false, false).unwrap();
        assert_eq!(d.encoding(), "utf-8");
        assert_eq!(d.decode(&[0x61, 0xE2, 0x82, 0xAC], false).unwrap(), "a€");
        assert_eq!(d.decode(&[], false).unwrap(), "");
    }

    #[test]
    fn labels_are_trimmed_and_case_insensitive() {
        assert_eq!(TextDecoder::new(" UTF-8\t", false, false).unwrap().encoding(), "utf-8");
        assert_eq!(
            TextDecoder::new("unicode-1-1-utf-8", false, false).unwrap().encoding(),
            "utf-8"
        );
        // The historical aliases resolve to windows-1252, not to real Latin1/ASCII
        assert_eq!(
            TextDecoder::new("latin1", false, false).unwrap().encoding(),
            "windows-1252"
        );
        assert_eq!(
            TextDecoder::new("ascii", false, false).unwrap().encoding(),
            "windows-1252"
        );
    }

    #[test]
    fn unknown_and_replacement_labels_fail() {
        let err = TextDecoder::new("bogus-encoding", false, false).unwrap_err();
        assert!(err.to_string().starts_with("RangeError:"));
        // Labels of the replacement encoding must fail like unknown ones
        assert!(TextDecoder::new("csiso2022kr", false, false).is_err());
        assert!(TextDecoder::new("replacement", false, false).is_err());
    }

    #[test]
    fn replacement_vs_fatal() {
        let mut lossy = TextDecoder::new("utf-8", false, false).unwrap();
        assert_eq!(lossy.decode(&[0xFF], false).unwrap(), "\u{FFFD}");

        let mut fatal = TextDecoder::new("utf-8", true, false).unwrap();
        assert_eq!(fatal.decode(&[0xFF], false), Err(EncodingError::Malformed));
    }

    #[test]
    fn bom_handling() {
        let mut d = TextDecoder::new("utf-8", false, false).unwrap();
        assert_eq!(d.decode(&[0xEF, 0xBB, 0xBF, 0x61], false).unwrap(), "a");

        let mut keep = TextDecoder::new("utf-8", false, true).unwrap();
        assert_eq!(keep.decode(&[0xEF, 0xBB, 0xBF, 0x61], false).unwrap(), "\u{FEFF}a");
    }

    #[test]
    fn utf16le() {
        let mut d = TextDecoder::new("utf-16le", false, false).unwrap();
        assert_eq!(d.encoding(), "utf-16le");
        assert_eq!(d.decode(&[0x61, 0x00, 0xAC, 0x20], false).unwrap(), "a€");
    }

    #[test]
    fn streaming_across_chunks() {
        let mut d = TextDecoder::new("utf-8", false, false).unwrap();
        // "€" split down the middle of its three-byte sequence
        assert_eq!(d.decode(&[0xE2], true).unwrap(), "");
        assert_eq!(d.decode(&[0x82, 0xAC], false).unwrap(), "€");

        // The non-stream call above flushed: the decoder must be fresh again
        assert_eq!(d.decode(&[0x61], false).unwrap(), "a");
    }

    #[test]
    fn empty_mid_stream_push_keeps_pending_lead_byte() {
        // Regression guard for encoding_rs#126: [0xFE] + [] + [0x40] must
        // still decode as one big5 character
        let mut d = TextDecoder::new("big5", false, false).unwrap();
        assert_eq!(d.decode(&[0xFE], true).unwrap(), "");
        assert_eq!(d.decode(&[], true).unwrap(), "");
        assert_eq!(d.decode(&[0x40], true).unwrap(), "\u{9442}");
        assert_eq!(d.decode(&[], false).unwrap(), "");
    }

    #[test]
    fn incomplete_tail_is_flushed() {
        let mut d = TextDecoder::new("utf-8", false, false).unwrap();
        assert_eq!(d.decode(&[0x61, 0xE2], false).unwrap(), "a\u{FFFD}");

        let mut fatal = TextDecoder::new("utf-8", true, false).unwrap();
        assert_eq!(fatal.decode(&[0x61, 0xE2], false), Err(EncodingError::Malformed));
    }
}
