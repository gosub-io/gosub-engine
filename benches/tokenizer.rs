use criterion::{criterion_group, criterion_main, Criterion};
use gosub_engine::testing::tokenizer;

fn criterion_benchmark(c: &mut Criterion) {
    // Criterion can report inconsistent results from run to run in some cases.  We attempt to
    // minimize that in this setup.
    // https://stackoverflow.com/a/74136347/61048
    let mut group = c.benchmark_group("tokenization");
    group.significance_level(0.1).sample_size(500);

    // Fetch the files outside of the closure to avoid issues with file io
    let fixtures = tokenizer::fixtures().collect::<Vec<_>>();

    group.bench_function("fixtures", |b| {
        b.iter(|| {
            for root in &fixtures {
                for test in &root.tests {
                    test.tokenize();
                }
            }
        })
    });

    group.finish();
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
