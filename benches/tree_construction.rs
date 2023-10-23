use criterion::{criterion_group, criterion_main, Criterion};
use gosub_engine::testing::tree_construction;

fn criterion_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("Tree construction");
    group.significance_level(0.1).sample_size(500);

    // Careful about reading files inside the closure
    let filenames = Some(&["tests1.dat"][..]);
    let fixtures = tree_construction::fixtures(filenames).expect("problem loading fixtures");

    group.bench_function("fixtures", |b| {
        b.iter(|| {
            for root in &fixtures {
                for test in &root.tests {
                    let _ = test.parse().unwrap();
                }
            }
        })
    });

    group.finish();
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
