.SILENT:

SHELL=/usr/bin/env bash -O globstar

all: help

test: test_unit test_clippy test_fmt test_commands ## Runs tests

bench: ## Benchmark the project
	cargo bench

build: ## Build the project
	source test-utils.sh ;\
	section "Cargo build" ;\
	cargo build --all

fix-format:  ## Fix formatting and clippy errors
	cargo fmt --all
	cargo clippy --all --fix --allow-dirty --allow-staged

check-format: test_clippy test_fmt ## Check the project for clippy and formatting errors

test_unit:
	source test-utils.sh ;\
	section "Cargo nextest" ;\
	if cargo nextest --version >/dev/null 2>&1; then \
		cargo nextest run --all --no-fail-fast; \
		cargo test --doc --all; \
	else \
		echo "cargo-nextest not found, falling back to cargo test (install with: cargo install cargo-nextest)" ;\
		cargo test --all --no-fail-fast --all-targets; \
	fi

test_clippy:
	source test-utils.sh ;\
	section "Cargo clippy" ;\
	cargo clippy -- -D warnings

test_fmt:
	source test-utils.sh ;\
	section "Cargo fmt" ;\
	cargo fmt --all -- --check

test_commands:
	cargo run --bin html5-parser-test >/dev/null
	cargo run --bin parser-test >/dev/null
	cargo run --bin config-store list >/dev/null
	cargo run --bin gosub-parser file://tests/data/tree_iterator/stackoverflow.html >/dev/null
	cargo run --example html5-parser >/dev/null

help: ## Display available commands
	echo "Available make commands:"
	echo
	grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'
