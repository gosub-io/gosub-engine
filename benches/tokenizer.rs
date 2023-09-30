use criterion::{black_box, criterion_group, criterion_main, Criterion};
use gosub_engine::testing::tokenizer::{self, Test};

fn tokenize(test: &Test) {
    for mut builder in test.builders() {
        let mut tokenizer = builder.build();

        // If there is no output, still do an (initial) next token so the parser can generate
        // errors.
        if test.output.is_empty() {
            tokenizer.next_token();
        }

        // There can be multiple tokens to match. Make sure we match all of them
        for _ in test.output.iter() {
            tokenizer.next_token();
        }
    }
}

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
                    tokenize(black_box(test))
                }
            }
        })
    });

    group.finish();
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
