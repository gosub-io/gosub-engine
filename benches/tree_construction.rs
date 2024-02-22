use criterion::{criterion_group, criterion_main, Criterion};
use gosub_testing::testing::tree_construction;
use gosub_testing::testing::tree_construction::Harness;

fn criterion_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("Tree construction");
    group.significance_level(0.1).sample_size(500);

    // Careful about reading files inside the closure
    let filenames = Some(&["tests1.dat"][..]);
    let fixtures =
        tree_construction::fixture::read_fixtures(filenames).expect("problem loading fixtures");

    let mut harness = Harness::new();

    group.bench_function("fixtures", |b| {
        b.iter(|| {
            for root in &fixtures {
                for test in &root.tests {
                    for &scripting_enabled in test.script_modes() {
                        let _ = harness.run_test(test.clone(), scripting_enabled);
                    }
                }
            }
        });
    });

    group.finish();
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
