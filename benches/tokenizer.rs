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
    c.bench_function("tokenization", |b| {
        b.iter(|| {
            for root in tokenizer::fixtures() {
                for test in &root.tests {
                    tokenize(black_box(test))
                }
            }
        })
    });
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
