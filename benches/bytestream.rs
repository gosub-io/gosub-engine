// Benchmark code: panicking on bad input is the desired behavior, as in any test code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use gosub_shared::byte_stream::{ByteStream, Encoding, Stream};

/// Build a ~`target` byte UTF-8 input by repeating the sample test file.
fn make_input(target: usize) -> String {
    let sample = std::fs::read_to_string("tests/data/bytestream/utf8.txt").unwrap();
    let mut s = String::with_capacity(target + sample.len());
    while s.len() < target {
        s.push_str(&sample);
    }
    s
}

/// Eager decode: how long `from_str` (buffer -> chars + offsets + line_starts) takes.
fn decode(c: &mut Criterion) {
    let mut group = c.benchmark_group("bytestream/decode");
    for &size in &[16 * 1024usize, 1024 * 1024, 8 * 1024 * 1024] {
        let input = make_input(size);
        group.throughput(Throughput::Bytes(input.len() as u64));
        group.bench_function(format!("{}KB", input.len() / 1024), |b| {
            b.iter(|| {
                let stream = ByteStream::from_str(black_box(&input), Encoding::UTF8);
                black_box(stream.tell_bytes());
            });
        });
    }
    group.finish();
}

/// Iteration: drain the whole (already-decoded) stream, resetting each run.
fn iterate(c: &mut Criterion) {
    let mut group = c.benchmark_group("bytestream/iterate");
    for &size in &[1024 * 1024usize, 8 * 1024 * 1024] {
        let input = make_input(size);
        let mut stream = ByteStream::from_str(&input, Encoding::UTF8);
        group.throughput(Throughput::Bytes(input.len() as u64));
        group.bench_function(format!("{}KB", input.len() / 1024), |b| {
            b.iter(|| {
                stream.reset_stream();
                while !stream.eof() {
                    black_box(stream.read_and_next());
                }
            });
        });
    }
    group.finish();
}

/// Streaming append: feed the input in fixed-size chunks via append_str.
/// Exposes the O(n^2) full re-decode on every append.
fn append_chunks(c: &mut Criterion) {
    let mut group = c.benchmark_group("bytestream/append");
    let input = make_input(1024 * 1024);
    const CHUNK: usize = 4096;
    group.throughput(Throughput::Bytes(input.len() as u64));
    group.sample_size(20);
    group.bench_function("1MB in 4KB chunks", |b| {
        b.iter(|| {
            let mut stream = ByteStream::new(Encoding::UTF8, None);
            let mut pos = 0;
            while pos < input.len() {
                let end = (pos + CHUNK).min(input.len());
                // chunk on a char boundary
                let end = (end..=input.len()).find(|&e| input.is_char_boundary(e)).unwrap();
                stream.append_str(black_box(&input[pos..end]));
                pos = end;
            }
            stream.close();
            black_box(stream.tell_bytes());
        });
    });
    group.finish();
}

criterion_group!(benches, decode, iterate, append_chunks);
criterion_main!(benches);
