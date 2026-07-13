# ByteStream

`gosub_shared::byte_stream::ByteStream` turns a buffer of raw bytes into a stream of decoded
characters, so parsers can read text without caring about the source encoding. It is the input
type for both the HTML5 tokenizer and the CSS3 parser. Bytes are transcoded eagerly into an
internal normalized WTF-8 text buffer (~1 byte per input byte; newline normalization applied
up front, lone UTF-16 surrogates kept via their WTF-8 encoding) and characters are decoded from
it on the fly. A line table plus incremental column cache reports exact `Location`s
(line/column/offset) for parse errors, and the original bytes are retained so the stream can
re-transcode mid-parse when a document declares its real encoding late (e.g. a `<meta charset>`
tag).

Reading yields a `Character`:

- `Ch(char)` — a decoded character
- `Surrogate(u16)` — an unpaired UTF-16 surrogate (0xD800–0xDFFF), which cannot be stored in a
  Rust `char`; it's up to the caller to deal with it
- `StreamEnd` — the read position is past the last decoded character

## Encodings

- `Unknown` — decodes nothing until a real encoding is set
- `Latin1` — each byte is one character
- `UTF8` — invalid sequences decode to U+FFFD (one replacement character per rejected sequence)
- `UTF16LE` / `UTF16BE` — unpaired surrogates come out as `Character::Surrogate`

## Creating and filling a stream

The one-shot constructor covers most uses — it decodes the string and closes the stream:

```rust
use gosub_shared::byte_stream::{ByteStream, Character, Encoding, Stream};

let mut stream = ByteStream::from_str("Hello", Encoding::UTF8);
assert_eq!(stream.read_and_next(), Character::Ch('H'));
assert_eq!(stream.look_ahead(2), Character::Ch('l'));
```

For other sources, build with `new(encoding, config)` and fill with `read_from_bytes(&[u8])`,
`read_from_file(impl Read)` (both return `io::Result` and close the stream), or
`read_from_str(&str, Option<Encoding>)` (leaves the stream open).

An **open** stream can still grow via `append_str`; an incomplete UTF-8 sequence at the end of an
open buffer is held back until more bytes arrive. `close()` marks the stream complete and
re-decodes, so a trailing incomplete sequence resolves to U+FFFD. Reading past the end returns
`StreamEnd` either way — use `eof()` (`closed() && exhausted()`) to tell "really finished" from
"waiting for more data".

## Reading

`ByteStream` implements the `Stream` trait:

- `read()` / `read_and_next()` — current character, without/with advancing
- `look_ahead(offset)` — peek `offset` characters ahead
- `next()` / `next_n(n)` / `prev()` / `prev_n(n)` — move the position
- `get_slice(len)` — the next `len` characters, without advancing
- `mark()` / `reset_to_mark(mark)` — save and restore a position (inherent methods)
- `seek_bytes(offset)` / `tell_bytes()` — position in bytes of the decoded text (UTF-8 space,
  not raw input bytes); a `tell_bytes` value can always be passed back to `seek_bytes`
- `reset_stream()` — back to the start
- `location()` — current `Location { line, column, offset }` (1-based line/column), amortized O(1)

## Configuration

`new` takes an optional `Config`:

- `cr_lf_as_one` (default `true`) — a CRLF pair is normalized to a single LF
- `replace_cr_as_lf` (default `false`) — a lone CR (not followed by LF) is normalized to LF
- `replace_high_ascii` (default `false`) — Latin1 only: bytes above 127 decode to `?`

## Detecting and switching the encoding

`detect_encoding()` sniffs a BOM first (UTF-8, UTF-16LE, UTF-16BE), then runs `chardetng` over at
most the first 64KB of the buffer; anything that isn't UTF-16 is reported as UTF-8. It only
returns the guess — apply it with `set_encoding(e)`, which re-transcodes the retained raw bytes
and preserves the current character index, so switching encodings mid-parse keeps your place.
