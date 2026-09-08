.SILENT:

SHELL=/usr/bin/env bash

.PHONY: all test bench build fix doc clean test-unit test-clippy test-fmt test-check test-smoke fuzz-html5 fuzz-html5-tokenizer test-deny ci-check fuzz-css3 help examples

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
		cargo run --example config-store -- list >/dev/null && \
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
	echo
	printf 'To run an example: \033[36mmake examples\033[0m lists them all with a ready-to-paste command.\n'

# ---------------------------------------------------------------------------
# Examples
#
# One table drives both the menu and the run-% rule. Columns: name, group,
# description. Adding a row here is all a new example needs; anything found on
# disk but missing from the table still shows up under "Undocumented".
# ---------------------------------------------------------------------------
define EXAMPLES_TABLE
hello-world     engine  Single tab navigating a URL, streaming every engine event to stdout
tutorial        engine  Minimal Engine -> Zone -> Tab -> Navigate lifecycle (see docs/tutorial.md)
multi-tab       engine  25 tabs navigating at once, with live per-tab progress bars
multi-process   engine  Browser-shaped embedder running the engine with process isolation
html5-parser    engine  Parse an HTML document with gosub_html5 directly and print the DOM
pipeline-test   engine  End-to-end smoke test against a tiny local HTTP server
config-store    engine  View and modify the configuration store (list, search, set, ...)
metrics-cli     engine  Timing stats from a running engine (--watch, --json, --reset)
winit-vello     gui     winit window, Vello/wgpu GPU rendering
winit-skia      gui     winit window, Skia CPU rendering
winit-skia-gpu  gui     winit window, Skia GPU (OpenGL) rendering
winit-cairo     gui     winit window, Cairo CPU rendering
gtk4-cairo      gui     GTK4 window, Cairo CPU rendering (Pango text)
gtk4-skia       gui     GTK4 window, Skia CPU rendering
gtk4-skia-gpu   gui     GTK4 window, Skia GPU (OpenGL/GLArea) rendering
egui-vello      gui     egui window, Vello/wgpu GPU rendering
egui-skia       gui     egui window, Skia CPU rendering
egui-cairo      gui     egui window, Cairo CPU rendering
mini-browser    gui     Every process-isolation setting on; Ctrl+P prints the process tree
endef
export EXAMPLES_TABLE

# Every example target that actually exists: [[example]] names in the root
# Cargo.toml, plus one package per examples/<name>/ directory.
define discover_examples
{ awk '/^\[\[example\]\]/{g=1;next} g&&/^name/{if(match($$0,/"[^"]+"/))print substr($$0,RSTART+1,RLENGTH-2);g=0}' Cargo.toml; \
  find examples -mindepth 2 -maxdepth 2 -name Cargo.toml -printf '%h\n' 2>/dev/null | xargs -r -n1 basename; } | sort -u
endef

examples: ## List the runnable examples and how to start each one
	found=$$($(discover_examples)) ;\
	list() { echo "$$EXAMPLES_TABLE" | awk -v found="$$found" -v grp="$$1" \
		'BEGIN{n=split(found,a,"\n");for(i=1;i<=n;i++)have[a[i]]=1} \
		 $$2==grp && have[$$1] {n2=$$1;$$1="";$$2="";sub(/^ +/,""); \
		 printf "  \033[36mmake run-%-16s\033[0m %s\n",n2,$$0}' ; } ;\
	printf '\033[1mEngine examples\033[0m  (headless, no GUI or extra system packages)\n\n' ;\
	list engine ;\
	printf '\n\033[1mGUI examples\033[0m  (open a window; need GTK4/Cairo/Skia system libs)\n\n' ;\
	list gui ;\
	known=$$(echo "$$EXAMPLES_TABLE" | awk 'NF{print $$1}' | sort -u) ;\
	extra=$$(comm -13 <(echo "$$known") <(echo "$$found")) ;\
	if [ -n "$$extra" ]; then \
		printf '\n\033[1mUndocumented\033[0m  (found on disk, missing from the Makefile table)\n\n' ;\
		echo "$$extra" | sed 's/^/  \x1b[36mmake run-/;s/$$/\x1b[0m/' ;\
	fi ;\
	printf '\n\033[1mUsage\033[0m\n\n' ;\
	printf '  make run-<name>                             run it\n' ;\
	printf '  make run-winit-vello URL=https://gosub.io   pass a URL\n' ;\
	printf '  make run-config-store ARGS="list"           pass arbitrary arguments\n' ;\
	printf '  make run-winit-vello RELEASE=1              build optimised (recommended for GUI)\n\n'

# CARGO is overridable so the dispatch can be exercised without a real build.
CARGO ?= cargo

# RELEASE=1 -> --release; URL/ARGS are forwarded to the example itself.
run-%:
	name='$*' ;\
	relflag='' ; [ -n "$(RELEASE)" ] && relflag='--release' ;\
	if [ -f "examples/$$name/Cargo.toml" ]; then \
		set -- $(CARGO) run $$relflag -p "example-$$name" -- $(URL) $(ARGS) ;\
	elif $(discover_examples) | grep -qx "$$name"; then \
		set -- $(CARGO) run $$relflag --example "$$name" -- $(URL) $(ARGS) ;\
	else \
		printf '\033[31mUnknown example: %s\033[0m\n\n' "$$name" >&2 ;\
		$(MAKE) --no-print-directory examples >&2 ;\
		exit 1 ;\
	fi ;\
	printf '\033[90m$$ %s\033[0m\n' "$$*" ;\
	exec "$$@"
