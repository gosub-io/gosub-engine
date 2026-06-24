.SILENT:

SHELL=/usr/bin/env bash

.PHONY: all test bench build fix doc clean test-unit test-clippy test-fmt test-check test-smoke fuzz-html5 fuzz-html5-tokenizer test-deny ci-check fuzz-css3 help

all: help

test: test-fmt test-clippy test-smoke test-unit test-check ## Run all checks (fmt + clippy + smoke + unit + check)

bench: ## Benchmark the project
	cargo bench

build: ## Build all crates
	source test-utils.sh ;\
	run_section "Cargo build" cargo build --all

fix: ## Auto-fix formatting and clippy warnings
	cargo fmt --all
	cargo clippy --all --fix --allow-dirty --allow-staged

doc: ## Build crate documentation
	cargo doc --no-deps --all

clean: ## Remove build artifacts
	cargo clean

test-unit: ## Run unit and doc tests
	source test-utils.sh ;\
	run_section "Unit tests" bash -c '\
		if cargo nextest --version >/dev/null 2>&1; then \
			cargo nextest run --all --no-fail-fast && cargo test --doc --all; \
		else \
			echo "cargo-nextest not found, falling back (install: cargo install cargo-nextest)" ;\
			cargo test --all --no-fail-fast --all-targets; \
		fi \
	'

test-clippy: ## Check for clippy warnings
	source test-utils.sh ;\
	run_section "Cargo clippy" cargo clippy --locked --all --all-targets -- -D warnings

test-fmt: ## Check formatting
	source test-utils.sh ;\
	run_section "Cargo fmt" cargo fmt --all -- --check

test-check: ## Check all features compile against locked dependencies
	source test-utils.sh ;\
	run_section "Cargo check" cargo check --locked --all --all-features

test-deny: ## Check dependencies for advisories, licenses, bans and sources
	source test-utils.sh ;\
	run_section "Cargo deny" bash -c '\
		if cargo deny --version >/dev/null 2>&1; then \
			cargo deny check; \
		else \
			echo "cargo-deny not found (install: cargo install --locked cargo-deny)" ;\
			exit 1; \
		fi \
	'

ci-check: test-fmt test-clippy test-check test-unit test-deny ## Run all CI checks (fmt + clippy + check-features + unit + deny)

test-smoke: ## CLI smoke tests
	source test-utils.sh ;\
	run_section "CLI smoke tests" bash -c '\
		cargo run --bin html5-parser-test >/dev/null && \
		cargo run --bin parser-test >/dev/null && \
		cargo run --bin config-store list >/dev/null && \
		cargo run --bin gosub-parser file://tests/data/tree_iterator/stackoverflow.html >/dev/null && \
		cargo run --example html5-parser >/dev/null && \
		cargo run --example pipeline-test \
	'

fuzz-html5: ## Run html5 parser fuzzer (cargo-fuzz, requires nightly)
	cd crates/gosub_html5 && cargo +nightly fuzz run html5_parser -- -dict=fuzz/html.dict

fuzz-html5-tokenizer: ## Run html5 tokenizer fuzzer (cargo-fuzz, requires nightly)
	cd crates/gosub_html5 && cargo +nightly fuzz run tokenizer -- -dict=fuzz/html.dict

fuzz-css3: ## Run CSS3 parser fuzzer (cargo-fuzz, requires nightly)
	cd crates/gosub_css3 && cargo +nightly fuzz run css3_parser -- -dict=fuzz/css3.dict

help: ## Display available commands
	echo "Available make commands:"
	echo
	grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'
