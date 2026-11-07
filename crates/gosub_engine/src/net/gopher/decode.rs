//! The item types that are a file sent as text: `6`, uuencoded, and `4`, a BinHex 4.0
//! Macintosh file (RFC 1741). Both are decoded as they arrive, so a large file streams like
//! any binary item. Of a BinHex file only the data fork is kept; the resource fork is read
//! for its CRC and dropped.
//!
//! Anything that does not decode is an error, never bytes: a file cut short, a character
//! outside the encoding, a CRC that does not match. An error found at the end of a stream
//! fails the body there, after its last bytes, so a download of it fails rather than finishes.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, ReadBuf};

/// Most bytes skipped before the encoded file begins: a server sending text that never
/// starts a file is not read forever.
const MAX_PREAMBLE: usize = 64 * 1024;

/// Longest line read before and inside a uuencoded file.
const MAX_LINE: usize = 1024;

/// How an item's file is encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Uuencode,
    BinHex,
}

impl Encoding {
    /// The encoding of an item type's files, for the types that have one.
    pub fn of(item_type: char) -> Option<Self> {
        match item_type {
            '6' => Some(Self::Uuencode),
            '4' => Some(Self::BinHex),
            _ => None,
        }
    }
}

/// Why a file did not decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeError(&'static str);

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for DecodeError {}

/// A decoder fed the encoded text in pieces of any size.
#[derive(Debug)]
pub enum Decoder {
    Uuencode(Uudecode),
    BinHex(Hexbin),
}

impl Decoder {
    pub fn new(encoding: Encoding) -> Self {
        match encoding {
            Encoding::Uuencode => Self::Uuencode(Uudecode::default()),
            Encoding::BinHex => Self::BinHex(Hexbin::default()),
        }
    }

    /// Decode `input`, appending the file's bytes to `out`. Input after the file is ignored.
    pub fn feed(&mut self, input: &[u8], out: &mut Vec<u8>) -> Result<(), DecodeError> {
        match self {
            Self::Uuencode(d) => d.feed(input, out),
            Self::BinHex(d) => d.feed(input, out),
        }
    }

    /// The input has ended: an error unless the whole file was in it.
    pub fn finish(&mut self, out: &mut Vec<u8>) -> Result<(), DecodeError> {
        match self {
            Self::Uuencode(d) => d.finish(out),
            Self::BinHex(d) => d.finish(),
        }
    }

    /// Whether the whole file is decoded.
    pub fn done(&self) -> bool {
        match self {
            Self::Uuencode(d) => d.state == UuState::Done,
            Self::BinHex(d) => d.fork == Fork::Done,
        }
    }

    /// The file's name, as encoded, once the decoder has read it.
    pub fn name(&self) -> Option<&[u8]> {
        match self {
            Self::Uuencode(d) => d.name.as_deref(),
            Self::BinHex(d) => d.name.as_deref(),
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum UuState {
    /// Before the `begin` line.
    #[default]
    Preamble,
    /// Data lines, up to the one of length zero.
    Data,
    /// After the line of length zero, before `end`.
    End,
    Done,
}

/// A uuencoded file: `begin <mode> <name>`, lines of a length character and groups of four
/// characters for three bytes, a line of length zero, `end`.
#[derive(Debug, Default)]
pub struct Uudecode {
    state: UuState,
    line: Vec<u8>,
    skipped: usize,
    name: Option<Vec<u8>>,
}

impl Uudecode {
    fn feed(&mut self, input: &[u8], out: &mut Vec<u8>) -> Result<(), DecodeError> {
        for &b in input {
            if self.state == UuState::Done {
                return Ok(());
            }
            if self.state == UuState::Preamble {
                self.skipped += 1;
                if self.skipped > MAX_PREAMBLE {
                    return Err(DecodeError("no uuencoded file in the item"));
                }
            }
            if b == b'\n' {
                self.take_line(out)?;
            } else if self.line.len() == MAX_LINE {
                return Err(DecodeError("a line of the uuencoded file is too long"));
            } else {
                self.line.push(b);
            }
        }
        Ok(())
    }

    fn finish(&mut self, out: &mut Vec<u8>) -> Result<(), DecodeError> {
        if !self.line.is_empty() && self.state != UuState::Done {
            self.take_line(out)?;
        }
        match self.state {
            UuState::Done => Ok(()),
            UuState::Preamble => Err(DecodeError("no uuencoded file in the item")),
            _ => Err(DecodeError("the uuencoded file is cut short")),
        }
    }

    fn take_line(&mut self, out: &mut Vec<u8>) -> Result<(), DecodeError> {
        let mut line = std::mem::take(&mut self.line);
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        match self.state {
            UuState::Preamble => {
                if line.starts_with(b"begin-base64") {
                    return Err(DecodeError("base64 in uuencode's frame is not supported"));
                }
                if let Some(name) = begin_line(&line) {
                    self.name = Some(name.to_vec());
                    self.state = UuState::Data;
                }
            }
            UuState::Data => {
                // A line of length zero is often sent with its space stripped.
                let Some(&first) = line.first() else {
                    self.state = UuState::End;
                    return Ok(());
                };
                let len = usize::from(uu_value(first)?);
                if len == 0 {
                    self.state = UuState::End;
                    return Ok(());
                }
                // Encoders strip trailing spaces, which are zero bits: a character missing at
                // the end is one. Characters past what the length needs are ignored.
                let chars = &line[1..];
                let mut at = 0;
                for group in 0..len.div_ceil(3) {
                    let mut bits = 0u32;
                    for i in 0..4 {
                        let c = chars.get(group * 4 + i).copied().unwrap_or(b' ');
                        bits = bits << 6 | u32::from(uu_value(c)?);
                    }
                    for byte in [(bits >> 16) as u8, (bits >> 8) as u8, bits as u8] {
                        if at < len {
                            out.push(byte);
                            at += 1;
                        }
                    }
                }
            }
            UuState::End => {
                if line.trim_ascii_end() != b"end" {
                    return Err(DecodeError("the uuencoded file has no end line"));
                }
                self.state = UuState::Done;
            }
            UuState::Done => {}
        }
        Ok(())
    }
}

/// The name in a `begin <octal mode> <name>` line.
fn begin_line(line: &[u8]) -> Option<&[u8]> {
    let rest = line.strip_prefix(b"begin ")?;
    let space = rest.iter().position(|&b| b == b' ')?;
    let (mode, name) = (&rest[..space], &rest[space + 1..]);
    if mode.is_empty() || !mode.iter().all(|b| (b'0'..=b'7').contains(b)) || name.is_empty() {
        return None;
    }
    Some(name)
}

/// The six bits a uuencode character stands for: ` ` to `` ` ``, with `` ` `` for zero.
fn uu_value(c: u8) -> Result<u8, DecodeError> {
    match c {
        b' '..=b'`' => Ok((c - b' ') & 0x3f),
        _ => Err(DecodeError("a character outside uuencode")),
    }
}

/// The 64 characters of BinHex 4.0, in the order of the values they stand for.
const HQX_ALPHABET: &[u8; 64] = b"!\"#$%&'()*+,-012345689@ABCDEFGHIJKLMNPQRSTUVXYZ[`abcdefhijklmpqr";

/// BinHex's run-length marker: `b 0x90 n` is `b` n times, `0x90 0x00` is 0x90 itself.
const RUN: u8 = 0x90;

/// Where in the decoded stream a BinHex file is.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Fork {
    /// The header, up to and including its CRC.
    #[default]
    Header,
    Data(u32),
    DataCrc,
    Resource(u32),
    ResourceCrc,
    Done,
}

/// A BinHex 4.0 file: text up to a line starting with `:`, then six bits a character, run
/// length encoded, holding a header, the data fork and the resource fork, each with a CRC.
#[derive(Debug, Default)]
pub struct Hexbin {
    started: bool,
    at_line_start: bool,
    skipped: usize,
    bits: u32,
    nbits: u8,
    last: Option<u8>,
    run: bool,
    fork: Fork,
    header: Vec<u8>,
    crc: u16,
    crc_bytes: Vec<u8>,
    resource_len: u32,
    name: Option<Vec<u8>>,
}

impl Hexbin {
    fn feed(&mut self, input: &[u8], out: &mut Vec<u8>) -> Result<(), DecodeError> {
        for &c in input {
            if self.fork == Fork::Done {
                return Ok(());
            }
            if !self.started {
                // The file starts at a `:` in the first column; anything before is ignored.
                self.skipped += 1;
                if self.skipped > MAX_PREAMBLE {
                    return Err(DecodeError("no BinHex file in the item"));
                }
                if c == b':' && (self.at_line_start || self.skipped == 1) {
                    self.started = true;
                }
                self.at_line_start = matches!(c, b'\r' | b'\n');
                continue;
            }
            let value = match c {
                b'\r' | b'\n' | b'\t' | b' ' => continue,
                b':' => return Err(DecodeError("the BinHex file is cut short")),
                _ => HQX_ALPHABET
                    .iter()
                    .position(|&a| a == c)
                    .ok_or(DecodeError("a character outside BinHex"))?,
            };
            self.bits = self.bits << 6 | value as u32;
            self.nbits += 6;
            if self.nbits >= 8 {
                self.nbits -= 8;
                let byte = (self.bits >> self.nbits) as u8;
                self.bits &= (1 << self.nbits) - 1;
                self.unrun(byte, out)?;
            }
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<(), DecodeError> {
        match self.fork {
            Fork::Done => Ok(()),
            _ if !self.started => Err(DecodeError("no BinHex file in the item")),
            _ => Err(DecodeError("the BinHex file is cut short")),
        }
    }

    /// Undo the run-length encoding of one byte. Runs cross the parts of the file.
    fn unrun(&mut self, byte: u8, out: &mut Vec<u8>) -> Result<(), DecodeError> {
        if self.run {
            self.run = false;
            if byte == 0 {
                self.last = Some(RUN);
                return self.take(RUN, out);
            }
            let repeated = self.last.ok_or(DecodeError("a BinHex run of nothing"))?;
            for _ in 1..byte {
                self.take(repeated, out)?;
            }
            Ok(())
        } else if byte == RUN {
            self.run = true;
            Ok(())
        } else {
            self.last = Some(byte);
            self.take(byte, out)
        }
    }

    /// One byte of the decoded file.
    fn take(&mut self, byte: u8, out: &mut Vec<u8>) -> Result<(), DecodeError> {
        match self.fork {
            Fork::Header => {
                self.header.push(byte);
                let name_len = usize::from(self.header[0]);
                if !(1..=63).contains(&name_len) {
                    return Err(DecodeError("a BinHex name must be 1 to 63 bytes"));
                }
                // Name length, name, version, type, creator, flags, data and resource
                // lengths; then the CRC of all that.
                let len = name_len + 20;
                if self.header.len() == len + 2 {
                    let crc = self.header[..len].iter().fold(0, |crc, &b| crc_step(crc, b));
                    if crc.to_be_bytes() != self.header[len..] {
                        return Err(DecodeError("the BinHex header's CRC does not match"));
                    }
                    let h = &self.header;
                    let field = |at: usize| u32::from_be_bytes([h[at], h[at + 1], h[at + 2], h[at + 3]]);
                    self.name = Some(self.header[1..=name_len].to_vec());
                    self.resource_len = field(len - 4);
                    self.next(Fork::Data(field(len - 8)));
                }
            }
            Fork::Data(left) => {
                out.push(byte);
                self.crc = crc_step(self.crc, byte);
                self.next(Fork::Data(left - 1));
            }
            Fork::Resource(left) => {
                self.crc = crc_step(self.crc, byte);
                self.next(Fork::Resource(left - 1));
            }
            Fork::DataCrc | Fork::ResourceCrc => {
                self.crc_bytes.push(byte);
                if self.crc_bytes.len() == 2 {
                    if self.crc.to_be_bytes() != self.crc_bytes[..] {
                        return Err(match self.fork {
                            Fork::DataCrc => DecodeError("the BinHex data fork's CRC does not match"),
                            _ => DecodeError("the BinHex resource fork's CRC does not match"),
                        });
                    }
                    self.crc_bytes.clear();
                    self.crc = 0;
                    self.next(match self.fork {
                        Fork::DataCrc => Fork::Resource(self.resource_len),
                        _ => Fork::Done,
                    });
                }
            }
            Fork::Done => {}
        }
        Ok(())
    }

    /// Move on to `fork`, past a fork with nothing left in it.
    fn next(&mut self, fork: Fork) {
        self.fork = match fork {
            Fork::Data(0) => Fork::DataCrc,
            Fork::Resource(0) => Fork::ResourceCrc,
            fork => fork,
        };
    }
}

/// One byte into BinHex's CRC: CCITT, polynomial 0x1021, starting at zero.
fn crc_step(crc: u16, byte: u8) -> u16 {
    let mut crc = crc ^ u16::from(byte) << 8;
    for _ in 0..8 {
        crc = if crc & 0x8000 != 0 { crc << 1 ^ 0x1021 } else { crc << 1 };
    }
    crc
}

/// Reads from the encoded file read from `inner` decoded. Ends once the file is whole, and
/// fails where it does not decode, after the bytes decoded before.
pub struct Decoding<R> {
    inner: R,
    decoder: Decoder,
    out: Vec<u8>,
    at: usize,
    buf: Box<[u8]>,
    failed: Option<DecodeError>,
}

impl<R: AsyncRead + Unpin> Decoding<R> {
    pub fn new(encoding: Encoding, inner: R) -> Self {
        Self {
            inner,
            decoder: Decoder::new(encoding),
            out: Vec::new(),
            at: 0,
            buf: vec![0; 16 * 1024].into_boxed_slice(),
            failed: None,
        }
    }

    /// The file's name, once it has been read.
    pub fn name(&self) -> Option<&[u8]> {
        self.decoder.name()
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for Decoding<R> {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, dst: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let this = &mut *self;
        // Reads that decode to nothing (a preamble, a resource fork) yield now and then.
        for _ in 0..16 {
            if this.at < this.out.len() {
                let n = dst.remaining().min(this.out.len() - this.at);
                dst.put_slice(&this.out[this.at..this.at + n]);
                this.at += n;
                return Poll::Ready(Ok(()));
            }
            if let Some(e) = this.failed {
                return Poll::Ready(Err(io::Error::new(io::ErrorKind::InvalidData, e)));
            }
            if this.decoder.done() {
                return Poll::Ready(Ok(()));
            }
            this.out.clear();
            this.at = 0;
            let mut read = ReadBuf::new(&mut this.buf);
            match Pin::new(&mut this.inner).poll_read(cx, &mut read) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Ready(Ok(())) => {}
            }
            let decoded = match read.filled() {
                [] => this.decoder.finish(&mut this.out),
                input => this.decoder.feed(input, &mut this.out),
            };
            // What decoded before the error is served first; the error follows.
            if let Err(e) = decoded {
                this.failed = Some(e);
            }
        }
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}

/// Encoders for tests: plain, no runs and no line limits, but files the decoders must take.
#[cfg(test)]
pub(super) mod encode {
    use super::{crc_step, HQX_ALPHABET, RUN};

    pub fn uuencode(name: &str, bytes: &[u8]) -> Vec<u8> {
        let char_of = |v: u32| if v == 0 { b'`' } else { b' ' + v as u8 };
        let mut text = format!("begin 644 {name}\n").into_bytes();
        for line in bytes.chunks(45) {
            text.push(char_of(line.len() as u32));
            for group in line.chunks(3) {
                let mut three = [0u8; 3];
                three[..group.len()].copy_from_slice(group);
                let bits = u32::from(three[0]) << 16 | u32::from(three[1]) << 8 | u32::from(three[2]);
                text.extend([18, 12, 6, 0].map(|shift| char_of(bits >> shift & 0x3f)));
            }
            text.push(b'\n');
        }
        text.extend(b"`\nend\n");
        text
    }

    /// A BinHex file holding `data` and `resource`, with its data fork CRC changed when
    /// `bad_crc`.
    pub fn binhex(name: &str, data: &[u8], resource: &[u8], bad_crc: bool) -> Vec<u8> {
        let crc = |bytes: &[u8]| bytes.iter().fold(0, |crc, &b| crc_step(crc, b)).to_be_bytes();
        let mut header = vec![name.len() as u8];
        header.extend(name.as_bytes());
        header.extend(b"\0TEXTttxt\0\0");
        header.extend((data.len() as u32).to_be_bytes());
        header.extend((resource.len() as u32).to_be_bytes());
        let mut raw = header.clone();
        raw.extend(crc(&header));
        raw.extend(data);
        let mut data_crc = crc(data);
        if bad_crc {
            data_crc[1] ^= 1;
        }
        raw.extend(data_crc);
        raw.extend(resource);
        raw.extend(crc(resource));
        let mut packed = Vec::new();
        for b in raw {
            packed.push(b);
            if b == RUN {
                packed.push(0);
            }
        }
        let mut text = b"(This file must be converted with BinHex 4.0)\n:".to_vec();
        for group in packed.chunks(3) {
            let mut three = [0u8; 3];
            three[..group.len()].copy_from_slice(group);
            let bits = u32::from(three[0]) << 16 | u32::from(three[1]) << 8 | u32::from(three[2]);
            let chars = [18, 12, 6, 0].map(|shift| HQX_ALPHABET[(bits >> shift & 0x3f) as usize]);
            text.extend(&chars[..group.len() + 1]);
        }
        text.extend(b":\n");
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    /// `bytes` decoded whole, fed one byte at a time and then all at once; both must agree.
    fn decode(encoding: Encoding, bytes: &[u8]) -> Result<(Vec<u8>, Vec<u8>), DecodeError> {
        let mut whole = Vec::new();
        let mut decoder = Decoder::new(encoding);
        let at_once = decoder
            .feed(bytes, &mut whole)
            .and_then(|()| decoder.finish(&mut whole))
            .map(|()| decoder.name().unwrap().to_vec());

        let mut split = Vec::new();
        let mut decoder = Decoder::new(encoding);
        let bytewise = bytes
            .iter()
            .try_for_each(|b| decoder.feed(std::slice::from_ref(b), &mut split))
            .and_then(|()| decoder.finish(&mut split))
            .map(|()| decoder.name().unwrap().to_vec());
        assert_eq!(at_once, bytewise);
        if at_once.is_ok() {
            assert_eq!(whole, split);
        }
        Ok((at_once?, whole))
    }

    /// 600 bytes with every value and runs in them.
    fn sample() -> Vec<u8> {
        let mut bytes: Vec<u8> = (0..=255u8).collect();
        bytes.extend([0x90; 40]);
        bytes.extend([7u8; 300]);
        bytes.extend([0x90, 0x00, 0x90]);
        bytes.extend(b"end.");
        bytes
    }

    // Made by Python's `binascii.b2a_uu`, which is an encoder independent of this one, from
    // `sample()`, with a mail header before it and a "." after.
    const UUENCODED: &[u8] = include_bytes!("testdata/sample.uue");

    // Made from `sample()` by a BinHex 4.0 encoder written from RFC 1741, CRCs by Python's
    // `binascii.crc_hqx`, with a resource fork of 70 bytes and the name `Sample: file/1`.
    const BINHEX: &[u8] = include_bytes!("testdata/sample.hqx");

    /// `BINHEX` with the character at `at` standing for a value 32 off: a change in the
    /// character's top bit, which is never padding.
    fn changed(at: usize) -> Vec<u8> {
        let mut text = BINHEX.to_vec();
        let value = HQX_ALPHABET.iter().position(|&a| a == text[at]).unwrap();
        text[at] = HQX_ALPHABET[value ^ 32];
        text
    }

    #[test]
    fn a_uuencoded_file_decodes_byte_for_byte() {
        let (name, bytes) = decode(Encoding::Uuencode, UUENCODED).unwrap();
        assert_eq!(name, b"sample file.bin");
        assert_eq!(bytes, sample());
    }

    #[test]
    fn a_binhex_file_decodes_to_its_data_fork() {
        let (name, bytes) = decode(Encoding::BinHex, BINHEX).unwrap();
        assert_eq!(name, b"Sample: file/1");
        assert_eq!(bytes, sample());
    }

    #[test]
    fn uuencode_with_backquotes_and_stripped_spaces_decodes() {
        // "Cat" and a zero-length line written with a backquote; then three bytes of zero
        // whose spaces were stripped.
        let text = b"begin 600 a\n#0V%T\n`\nend\n";
        assert_eq!(decode(Encoding::Uuencode, text).unwrap().1, b"Cat");
        let text = b"begin 600 a\r\n#\r\n\r\nend";
        assert_eq!(decode(Encoding::Uuencode, text).unwrap().1, [0, 0, 0]);
    }

    #[test]
    fn a_cut_short_uuencoded_file_is_refused() {
        let cut = &UUENCODED[..UUENCODED.len() / 2];
        assert_eq!(
            decode(Encoding::Uuencode, cut).unwrap_err(),
            DecodeError("the uuencoded file is cut short")
        );
        // Every line there, but not `end`.
        let text = b"begin 600 a\n#0V%T\n`\n";
        assert!(decode(Encoding::Uuencode, text).is_err());
    }

    #[test]
    fn uuencode_with_a_foreign_character_is_refused() {
        let text = b"begin 600 a\n#0V%t\n`\nend\n";
        assert_eq!(
            decode(Encoding::Uuencode, text).unwrap_err(),
            DecodeError("a character outside uuencode")
        );
    }

    #[test]
    fn text_that_is_no_uuencoded_file_is_refused() {
        assert!(decode(Encoding::Uuencode, b"just some text\n").is_err());
        assert!(decode(Encoding::Uuencode, b"begin 6x4 a\n#0V%T\n`\nend\n").is_err());
        assert!(decode(Encoding::Uuencode, b"begin-base64 644 a\nQ2F0\n====\n").is_err());
        assert!(decode(Encoding::Uuencode, &vec![b'\n'; MAX_PREAMBLE + 1]).is_err());
        assert!(decode(Encoding::Uuencode, &vec![b'x'; MAX_LINE + 1]).is_err());
    }

    #[test]
    fn a_binhex_file_with_a_changed_byte_is_refused() {
        // Each part's CRC catches a change in it.
        let start = BINHEX.iter().position(|&b| b == b':').unwrap() + 1;
        let end = BINHEX.iter().rposition(|&b| b == b':').unwrap();
        for at in [start + 2, start + 40, end - 30, end - 2] {
            let err = decode(Encoding::BinHex, &changed(at)).unwrap_err();
            assert!(err.0.contains("CRC"), "at {at}: {err}");
        }
    }

    #[test]
    fn a_cut_short_binhex_file_is_refused() {
        let cut = &BINHEX[..BINHEX.len() - 10];
        assert!(decode(Encoding::BinHex, cut).is_err());
        let mut early_colon = cut.to_vec();
        early_colon.extend(b":\n");
        assert_eq!(
            decode(Encoding::BinHex, &early_colon).unwrap_err(),
            DecodeError("the BinHex file is cut short")
        );
    }

    #[test]
    fn text_that_is_no_binhex_file_is_refused() {
        assert!(decode(Encoding::BinHex, b"(This file must be converted with BinHex 4.0)\n").is_err());
        assert_eq!(
            decode(Encoding::BinHex, b":7O!\n").unwrap_err(),
            DecodeError("a character outside BinHex")
        );
        assert!(decode(Encoding::BinHex, &vec![b' '; MAX_PREAMBLE + 1]).is_err());
    }

    #[test]
    fn what_the_test_encoders_make_decodes() {
        let data = sample();
        let text = encode::uuencode("u", &data);
        assert_eq!(
            decode(Encoding::Uuencode, &text).unwrap(),
            (b"u".to_vec(), data.clone())
        );
        let text = encode::binhex("h", &data, b"rsrc", false);
        assert_eq!(decode(Encoding::BinHex, &text).unwrap(), (b"h".to_vec(), data.clone()));
        let text = encode::binhex("h", &data, b"", true);
        assert!(decode(Encoding::BinHex, &text).is_err());
    }

    #[test]
    fn binhex_crc_is_ccitt_from_zero() {
        // CRC-16/XMODEM's check value.
        assert_eq!(b"123456789".iter().fold(0, |crc, &b| crc_step(crc, b)), 0x31c3);
    }

    #[tokio::test]
    async fn a_decoding_reader_serves_the_file_and_then_its_error() {
        let mut whole = Vec::new();
        Decoding::new(Encoding::BinHex, BINHEX)
            .read_to_end(&mut whole)
            .await
            .unwrap();
        assert_eq!(whole, sample());

        // A CRC that fails at the end fails the read after the data, never ends it cleanly.
        let changed = changed(BINHEX.iter().rposition(|&b| b == b':').unwrap() - 2);
        let mut reader = Decoding::new(Encoding::BinHex, &changed[..]);
        let mut got = Vec::new();
        let err = reader.read_to_end(&mut got).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("CRC"), "{err}");
    }
}
