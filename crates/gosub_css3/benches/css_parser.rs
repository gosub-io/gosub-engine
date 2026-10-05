//! Benchmarks for the CSS3 tokenizer and parser.
//!
//! Run: cargo bench -p gosub_css3 --bench css_parser
//!
//! Two stages are measured separately so a regression can be attributed:
//! - `tokenize`: drive the tokenizer over the whole input, no tree building
//! - `parse`: full `Css3::parse_str` producing a `CssStylesheet`
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::hint::black_box;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use gosub_css3::tokenizer::Tokenizer;
use gosub_css3::Css3;
use gosub_interface::css3::CssOrigin;
use gosub_shared::byte_stream::{ByteStream, Encoding, Location};
use gosub_shared::config::ParserConfig;

/// The compiled-in user agent stylesheet (~36 KiB, hand-written CSS).
const USERAGENT_CSS: &str = include_str!("../resources/useragent.css");

/// A large blob of concatenated real-world CSS (~2.2 MiB).
fn load_data_css() -> String {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/data/css3-data/data.css");
    std::fs::read_to_string(path).expect("tests/data/css3-data/data.css should exist")
}

fn tokenize_all(data: &str) -> usize {
    let mut stream = ByteStream::from_str(data, Encoding::UTF8);
    let mut tokenizer = Tokenizer::new(&mut stream, Location::default());

    let mut count = 0;
    while !tokenizer.eof() {
        black_box(tokenizer.consume());
        count += 1;
    }
    count
}

fn parse_all(data: &str) {
    let config = ParserConfig {
        ignore_errors: true,
        ..Default::default()
    };
    black_box(Css3::parse_str(data, config, CssOrigin::Author, "bench.css").expect("stylesheet should parse"));
}

fn bench_inputs(c: &mut Criterion) {
    let data_css = load_data_css();
    let inputs: [(&str, &str); 2] = [("useragent-36k", USERAGENT_CSS), ("real-world-2.2m", &data_css)];

    let mut group = c.benchmark_group("tokenize");
    for (name, css) in inputs {
        group.throughput(Throughput::Bytes(css.len() as u64));
        group.bench_function(name, |b| b.iter(|| tokenize_all(black_box(css))));
    }
    group.finish();

    let mut group = c.benchmark_group("parse");
    group.sample_size(20).measurement_time(Duration::from_secs(15));
    for (name, css) in inputs {
        group.throughput(Throughput::Bytes(css.len() as u64));
        group.bench_function(name, |b| b.iter(|| parse_all(black_box(css))));
    }
    group.finish();
}

criterion_group!(benches, bench_inputs);
criterion_main!(benches);
