# Bytestream 

A bytestream allows you to read characters from a series of bytes without worrying about their encoding. For instance,
a bytestream can have a series of bytes that represent a UTF-16 encoded string, but you can read it as if it were a
UTF-8 encoded string. The bytestream will take care of any conversions from the actual encoding into the actual output 
of the bytestream a `Character` enum.

Note that a bytestream can either be open or closed. When a stream is open, it is allowed to add more bytes to it. When
you have read all the bytes from the stream, it will return a `Character::StreamEmpty`. At this point you can either fill
up the stream with more bytes, or close the stream. Once a stream is closed, it will not accept any more bytes and reading
at the end of the stream will return a `Character::StreamEnd`.

## Encodings
The `bytestream` can handle the following encodings:

- UTF-8 (1-4 characters)
- UTF-16 Big Endian (2 characters)
- UTF-16 Little Endian (2 characters)
- ASCII (1 character)

When you read into the stream, the stream will return the next character based on the bytes in the stream. 

## Examples
```rust
use bytestream::{ByteStream, Encoding, Config};

fn main() {

    let mut stream = ByteStream::new(
        Encoding::UTF8,
        Some(Config {
            cr_lf_as_one: true,
            replace_high_ascii: false,
        }),
    );
    stream.read_from_bytes(&[0x48, 0x65, 0x6C, 0x6C, 0x6F]); // "Hello"
    stream.close();

    assert_eq!(stream.read_and_next(), Character::Char('H'));
    assert_eq!(stream.read_and_next(), Character::Char('e'));
    assert_eq!(stream.read_and_next(), Character::Char('l'));
    assert_eq!(stream.read_and_next(), Character::Char('l'));
    assert_eq!(stream.read_and_next(), Character::Char('o'));
    assert_eq!(stream.read_and_next(), Character::StreamEnd);
}
```

Note that in theory it's possible to switch encoding during the reading of the bytestream. The read functions will try and 
read the next bytes as the given encoding. We strongly advice you to not do this, as it can lead to unexpected results.

## Dealing with surrogates
Rust characters are UTF8 encoded and do not allow surrogate characters (0xD800 - 0xDFFF). If you try to read a surrogate 
character you will get a `Character::Surrogate`. It's up to the caller to deal with this if needed.


## Configuration settings
It's possible to add a configuration to the bytestream. This will set certain settings for the bytestream. The following
settings are available:

    - cr_lf_as_one: bool,
    This will treat a CR LF sequence as one character and will return only LF. By default, a CR LF sequence is treated as two characters.
 
    - replace_high_ascii: bool,
    If high-ascii (> 127) characters are found, they will be replaced with a `?` character. By default, high-ascii characters are not replaced.


## Detecting the encoding
It's possible to detect the encoding of a bytestream. This can be done by calling the `detect_encoding` function. This function
will return the detected encoding which you can manually set.

Note that the encoder detector will only work on the first 64Kb of bytes in the bytestream.