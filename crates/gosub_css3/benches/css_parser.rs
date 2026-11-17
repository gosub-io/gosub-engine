//! Benchmarks for the CSS3 tokenizer and parser, covering both CPU time and heap allocations.
//!
//! Run timing:    cargo bench -p gosub_css3 --bench css_parser
//! Run allocs:    cargo bench -p gosub_css3 --bench css_parser -- alloc-
//!
//! Stages are measured separately so a regression can be attributed:
//! - `tokenize`: drive the tokenizer over the whole input, no tree building
//! - `parse`: full `Css3::parse_str` producing a `CssStylesheet`
//! - `expand` (allocs only): `CssRule::expanded` on every rule of an already parsed sheet, the
//!   validation and shorthand expansion a rule pays the first time an element matches it
//!
//! The allocation benchmarks use a custom global allocator defined in this file
//! (applies only to this benchmark binary).
#![allow(clippy::unwrap_used, clippy::expect_used, unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use criterion::measurement::{Measurement, ValueFormatter};
use criterion::{criterion_group, criterion_main, BatchSize, Criterion, Throughput};
use gosub_css3::stylesheet::CssStylesheet;
use gosub_css3::tokenizer::Tokenizer;
use gosub_css3::Css3;
use gosub_interface::css3::CssOrigin;
use gosub_shared::byte_stream::{ByteStream, Encoding, Location};
use gosub_shared::config::ParserConfig;

// ── Counting allocator ────────────────────────────────────────────────────────

/// Total allocation calls (monotonically increasing; never decremented).
static ALLOC_OPS: AtomicU64 = AtomicU64::new(0);
/// Total bytes allocated (including realloc growth; never decremented).
static ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);

struct CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOC_OPS.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOC_OPS.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if new_size > layout.size() {
            ALLOC_OPS.fetch_add(1, Ordering::Relaxed);
            ALLOC_BYTES.fetch_add((new_size - layout.size()) as u64, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

// ── Custom criterion measurements ─────────────────────────────────────────────

struct CountFormatter(&'static str);
impl ValueFormatter for CountFormatter {
    fn scale_values(&self, _typical: f64, _values: &mut [f64]) -> &'static str {
        self.0
    }
    fn scale_throughputs(&self, _typical: f64, _throughput: &Throughput, _values: &mut [f64]) -> &'static str {
        self.0
    }
    fn scale_for_machines(&self, _values: &mut [f64]) -> &'static str {
        self.0
    }
}

struct AllocOps;
impl Measurement for AllocOps {
    type Intermediate = u64;
    type Value = u64;
    fn start(&self) -> u64 {
        ALLOC_OPS.load(Ordering::SeqCst)
    }
    fn end(&self, i: u64) -> u64 {
        ALLOC_OPS.load(Ordering::SeqCst).saturating_sub(i)
    }
    fn add(&self, a: &u64, b: &u64) -> u64 {
        a + b
    }
    fn zero(&self) -> u64 {
        0
    }
    fn to_f64(&self, v: &u64) -> f64 {
        *v as f64
    }
    fn formatter(&self) -> &dyn ValueFormatter {
        &CountFormatter("alloc-ops")
    }
}

struct AllocBytes;
impl Measurement for AllocBytes {
    type Intermediate = u64;
    type Value = u64;
    fn start(&self) -> u64 {
        ALLOC_BYTES.load(Ordering::SeqCst)
    }
    fn end(&self, i: u64) -> u64 {
        ALLOC_BYTES.load(Ordering::SeqCst).saturating_sub(i)
    }
    fn add(&self, a: &u64, b: &u64) -> u64 {
        a + b
    }
    fn zero(&self) -> u64 {
        0
    }
    fn to_f64(&self, v: &u64) -> f64 {
        *v as f64
    }
    fn formatter(&self) -> &dyn ValueFormatter {
        &CountFormatter("alloc-bytes")
    }
}

// ── Inputs and helpers ────────────────────────────────────────────────────────

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

fn parse_sheet(data: &str) -> CssStylesheet {
    let config = ParserConfig {
        ignore_errors: true,
        ..Default::default()
    };
    Css3::parse_str(data, config, CssOrigin::Author, "bench.css").expect("stylesheet should parse")
}

fn parse_all(data: &str) {
    black_box(parse_sheet(data));
}

/// Expansion is cached on the rule, so every iteration needs a sheet nothing has expanded yet.
fn expand_all(sheet: &CssStylesheet) -> usize {
    sheet.rules.iter().map(|rule| black_box(rule.expanded()).len()).sum()
}

// ── CPU timing benchmarks ─────────────────────────────────────────────────────

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

// ── Allocation benchmarks ─────────────────────────────────────────────────────

/// The counts are exact, so ten samples are plenty; the time only has to fit them.
fn bench_allocs<M: Measurement>(c: &mut Criterion<M>, kind: &str) {
    let data_css = load_data_css();
    let inputs: [(&str, &str); 2] = [("useragent-36k", USERAGENT_CSS), ("real-world-2.2m", &data_css)];

    let mut group = c.benchmark_group(format!("{kind}/tokenize"));
    group.sample_size(10).warm_up_time(Duration::from_secs(1));
    for (name, css) in inputs {
        group.bench_function(name, |b| b.iter(|| tokenize_all(black_box(css))));
    }
    group.finish();

    let mut group = c.benchmark_group(format!("{kind}/parse"));
    group.sample_size(10).warm_up_time(Duration::from_secs(1));
    for (name, css) in inputs {
        group.bench_function(name, |b| b.iter(|| parse_all(black_box(css))));
    }
    group.finish();

    let mut group = c.benchmark_group(format!("{kind}/expand"));
    group.sample_size(10).warm_up_time(Duration::from_secs(1));
    for (name, css) in inputs {
        group.bench_function(name, |b| {
            b.iter_batched(|| parse_sheet(css), |sheet| expand_all(&sheet), BatchSize::PerIteration)
        });
    }
    group.finish();
}

fn bench_alloc_ops(c: &mut Criterion<AllocOps>) {
    bench_allocs(c, "alloc-ops");
}

fn bench_alloc_bytes(c: &mut Criterion<AllocBytes>) {
    bench_allocs(c, "alloc-bytes");
}

// ── Criterion groups ──────────────────────────────────────────────────────────

criterion_group!(cpu_benches, bench_inputs);

// `without_plots`: the alloc counters are exact, and criterion's KDE plotting
// panics on a zero-variance sample when a count is identical to its baseline.
criterion_group! {
    name = alloc_ops_benches;
    config = Criterion::default().with_measurement(AllocOps).without_plots();
    targets = bench_alloc_ops
}

criterion_group! {
    name = alloc_bytes_benches;
    config = Criterion::default().with_measurement(AllocBytes).without_plots();
    targets = bench_alloc_bytes
}

criterion_main!(cpu_benches, alloc_ops_benches, alloc_bytes_benches);
