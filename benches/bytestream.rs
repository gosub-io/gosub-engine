use std::fs::File;

use criterion::{criterion_group, criterion_main, Criterion};
use gosub_shared::byte_stream::{ByteStream, Encoding, Stream};

fn utf8_testfile(c: &mut Criterion) {
    let mut group = c.benchmark_group("Bytestream test");
    group.significance_level(0.1).sample_size(500);

    let html_file = File::open("tests/data/bytestream/utf8.txt").unwrap();
    let mut stream = ByteStream::new(Encoding::UTF8, None);
    let _ = stream.read_from_file(html_file);
    stream.close();

    group.bench_function("utf8 test file", |b| {
        b.iter(|| {
            while !stream.eof() {
                stream.read_and_next();
            }
        })
    });

    group.finish();
}

criterion_group!(benches, utf8_testfile);
criterion_main!(benches);
