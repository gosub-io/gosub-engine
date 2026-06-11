//! Benchmarks for the HTML5 parser covering both CPU time and heap allocations.
//!
//! Run timing:    cargo bench -p gosub_html5 --bench html_parser
//! Run allocs:    cargo bench -p gosub_html5 --bench html_parser -- allocs
//!
//! The allocation benchmarks use a custom global allocator defined in this file
//! (applies only to this benchmark binary).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::io::Read as _;
use std::sync::atomic::{AtomicU64, Ordering};

use criterion::measurement::{Measurement, ValueFormatter};
use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use gosub_css3::system::Css3System;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::parser::Html5Parser;
use gosub_html5::{html_compile, testing::tree_construction};
use gosub_interface::config::{HasCssSystem, HasDocument, HasHtmlParser};
use std::time::Duration;

// ── Config ────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
struct Config;

impl HasCssSystem for Config {
    type CssSystem = Css3System;
}
impl HasDocument for Config {
    type Document = DocumentImpl<Self>;
}
impl HasHtmlParser for Config {
    type HtmlParser = Html5Parser<'static, Self>;
}

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

// ── HTML inputs ───────────────────────────────────────────────────────────────

/// Generates a ~150 KB synthetic HTML document that exercises the parser hot paths:
/// many elements with attributes, tables, deeply nested spans, SVG, and forms.
fn generate_large_html() -> String {
    let mut html = String::with_capacity(200_000);
    html.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n");
    html.push_str("<meta charset=\"UTF-8\">\n");
    html.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">\n");
    html.push_str("<title>Parser Benchmark Document</title>\n");
    html.push_str("</head>\n<body>\n");

    for section in 0..15_u32 {
        html.push_str(&format!(
            "<section id=\"s{section}\" class=\"content section\" data-index=\"{section}\" aria-label=\"Section {section}\">\n"
        ));
        html.push_str(&format!("<h2>Section {section} — Heading Text</h2>\n"));

        for para in 0..3_u32 {
            html.push_str(&format!("<p class=\"para\" data-para=\"{para}\">"));
            for _ in 0..8_u32 {
                html.push_str("The quick brown fox jumps over the lazy dog. ");
                html.push_str("Lorem ipsum dolor sit amet, consectetur adipiscing elit. ");
            }
            html.push_str("</p>\n");
        }

        // Table (exercises pending table character token handling)
        html.push_str("<table class=\"data-table\" border=\"1\">\n<thead><tr>");
        for col in 0..4_u32 {
            html.push_str(&format!("<th scope=\"col\">Column {col}</th>"));
        }
        html.push_str("</tr></thead>\n<tbody>\n");
        for row in 0..5_u32 {
            html.push_str("<tr>");
            for col in 0..4_u32 {
                html.push_str(&format!(
                    "<td data-row=\"{row}\" data-col=\"{col}\">Cell {row},{col}</td>"
                ));
            }
            html.push_str("</tr>\n");
        }
        html.push_str("</tbody></table>\n");

        // Nested spans (exercises active formatting elements)
        html.push_str("<p>");
        for depth in 0..5_u32 {
            html.push_str(&format!("<span class=\"d{depth}\">"));
        }
        html.push_str("Deeply nested content.");
        for _ in 0..5_u32 {
            html.push_str("</span>");
        }
        html.push_str("</p>\n</section>\n");
    }

    // SVG section (exercises SVG tag/attribute name adjustment)
    html.push_str("<div id=\"svg-section\">\n");
    for i in 0..20_u32 {
        let w = 100 + i;
        let h = 100 + i;
        html.push_str(&format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\">\n"
        ));
        html.push_str(
            "<rect x=\"10\" y=\"10\" width=\"80\" height=\"80\" fill=\"blue\" stroke=\"black\" stroke-width=\"2\"/>\n",
        );
        html.push_str("<circle cx=\"50\" cy=\"50\" r=\"30\" fill=\"red\" opacity=\"0.8\"/>\n");
        html.push_str("<text x=\"20\" y=\"20\" font-size=\"12\">SVG text content</text>\n");
        html.push_str("</svg>\n");
    }
    html.push_str("</div>\n");

    // Form (exercises attribute-heavy elements)
    html.push_str("<form id=\"bench-form\" method=\"post\" action=\"/submit\">\n");
    for i in 0..20_u32 {
        html.push_str(&format!(
            "<div class=\"field\"><label for=\"f{i}\">Field {i}</label>\
             <input type=\"text\" id=\"f{i}\" name=\"field{i}\" value=\"\" \
             placeholder=\"Enter value\" autocomplete=\"off\" required/></div>\n"
        ));
    }
    html.push_str("<button type=\"submit\" class=\"btn btn-primary\">Submit</button>\n</form>\n");

    html.push_str("</body>\n</html>");
    html
}

// ── Real-world HTML inputs ────────────────────────────────────────────────────

/// How many bytes of the WHATWG spec to feed the parser per benchmark iteration.
/// The full file is ~8 MB; at 512 KB one parse takes ~milliseconds even on the
/// unoptimised parser, giving criterion enough samples to produce stable numbers.
/// Raise this constant once the clone-heavy hot paths are fixed.
const WHATWG_BENCH_BYTES: usize = 512 * 1024;

/// Loads and truncates the WHATWG HTML Living Standard from
/// `resources/whatwg.html.gz` in the workspace root.
/// Decompressed once at startup; truncated at the last `>` before
/// `WHATWG_BENCH_BYTES` so we never cut inside a tag.
fn load_whatwg_spec() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/whatwg.html.gz");
    let file = std::fs::File::open(&path).unwrap_or_else(|e| panic!("failed to open {}: {e}", path.display()));
    let mut decoder = flate2::read::GzDecoder::new(file);
    let mut html = String::new();
    decoder
        .read_to_string(&mut html)
        .expect("failed to decompress whatwg.html.gz");

    if html.len() > WHATWG_BENCH_BYTES {
        // Truncate at the last `>` before the byte limit so we don't cut mid-tag.
        let cut = html[..WHATWG_BENCH_BYTES]
            .rfind('>')
            .map(|i| i + 1)
            .unwrap_or(WHATWG_BENCH_BYTES);
        html.truncate(cut);
    }
    html
}

// ── Benchmark helpers ─────────────────────────────────────────────────────────

fn parse_html(html: &str) {
    let _doc = html_compile::<Config>(black_box(html));
}

fn load_all_fixtures() -> Vec<tree_construction::fixture::FixtureFile> {
    tree_construction::fixture::read_fixtures(None).expect("failed to load fixtures")
}

fn parse_all_fixtures(fixtures: &[tree_construction::fixture::FixtureFile]) {
    use gosub_html5::testing::tree_construction::Harness;
    let mut harness = Harness::new();
    for root in fixtures {
        for test in &root.tests {
            for &scripting_enabled in test.script_modes() {
                let _ = harness.run_test::<Config>(test.clone(), scripting_enabled);
            }
        }
    }
}

// ── CPU timing benchmarks ─────────────────────────────────────────────────────

fn bench_cpu(c: &mut Criterion) {
    let large_html = generate_large_html();
    let fixtures = load_all_fixtures();
    let fixture_count: usize = fixtures.iter().map(|f| f.tests.len()).sum();
    let whatwg = load_whatwg_spec();

    let mut group = c.benchmark_group("parser/cpu");
    group.significance_level(0.05).warm_up_time(Duration::from_secs(2));

    // large_doc and whatwg_spec are slow on the unoptimised parser; keep
    // sample_size small so the full bench run stays under ~2 minutes.
    group.sample_size(10).measurement_time(Duration::from_secs(30));
    group.throughput(Throughput::Bytes(large_html.len() as u64));
    group.bench_function("large_doc", |b| {
        b.iter(|| parse_html(&large_html));
    });

    group.sample_size(30).measurement_time(Duration::from_secs(10));
    group.throughput(Throughput::Elements(fixture_count as u64));
    group.bench_function("all_fixtures", |b| {
        b.iter(|| parse_all_fixtures(&fixtures));
    });

    group.sample_size(10).measurement_time(Duration::from_secs(30));
    group.throughput(Throughput::Bytes(whatwg.len() as u64));
    group.bench_function("whatwg_spec", |b| {
        b.iter(|| parse_html(&whatwg));
    });

    group.finish();
}

// ── Allocation-ops benchmarks ─────────────────────────────────────────────────

fn bench_alloc_ops(c: &mut Criterion<AllocOps>) {
    let large_html = generate_large_html();
    let fixtures = load_all_fixtures();
    let whatwg = load_whatwg_spec();

    let mut group = c.benchmark_group("parser/alloc-ops");
    group.significance_level(0.05).warm_up_time(Duration::from_secs(1));

    group.sample_size(10).measurement_time(Duration::from_secs(30));
    group.bench_function("large_doc", |b| {
        b.iter(|| parse_html(&large_html));
    });
    group.sample_size(20).measurement_time(Duration::from_secs(10));
    group.bench_function("all_fixtures", |b| {
        b.iter(|| parse_all_fixtures(&fixtures));
    });
    group.sample_size(10).measurement_time(Duration::from_secs(30));
    group.bench_function("whatwg_spec", |b| {
        b.iter(|| parse_html(&whatwg));
    });

    group.finish();
}

// ── Allocation-bytes benchmarks ───────────────────────────────────────────────

fn bench_alloc_bytes(c: &mut Criterion<AllocBytes>) {
    let large_html = generate_large_html();
    let fixtures = load_all_fixtures();
    let whatwg = load_whatwg_spec();

    let mut group = c.benchmark_group("parser/alloc-bytes");
    group.significance_level(0.05).warm_up_time(Duration::from_secs(1));

    group.sample_size(10).measurement_time(Duration::from_secs(30));
    group.bench_function("large_doc", |b| {
        b.iter(|| parse_html(&large_html));
    });
    group.sample_size(20).measurement_time(Duration::from_secs(10));
    group.bench_function("all_fixtures", |b| {
        b.iter(|| parse_all_fixtures(&fixtures));
    });
    group.sample_size(10).measurement_time(Duration::from_secs(30));
    group.bench_function("whatwg_spec", |b| {
        b.iter(|| parse_html(&whatwg));
    });

    group.finish();
}

// ── Criterion groups ──────────────────────────────────────────────────────────

criterion_group!(cpu_benches, bench_cpu);

criterion_group! {
    name = alloc_ops_benches;
    config = Criterion::default().with_measurement(AllocOps);
    targets = bench_alloc_ops
}

criterion_group! {
    name = alloc_bytes_benches;
    config = Criterion::default().with_measurement(AllocBytes);
    targets = bench_alloc_bytes
}

criterion_main!(cpu_benches, alloc_ops_benches, alloc_bytes_benches);
