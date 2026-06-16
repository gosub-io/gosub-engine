# Development: tests and benchmarks

Run the test suite:

```bash
make test
```

Run the benchmarks (Criterion):

```bash
cargo bench
# open target/criterion/report/index.html
```

Build everything in the workspace, including all examples and GUI binaries:

```bash
cargo build --workspace --examples --bins
```

See [`examples.md`](examples.md) for the system packages some GUI backends require.
