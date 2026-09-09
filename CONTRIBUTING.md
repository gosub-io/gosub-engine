# Contributing to Gosub
Welcome and thanks for your interest in contributing to Gosub!

This is an initial (but not fully complete) contribution guide.

**Useful Links for Developers:**
* [Developer Chat](https://chat.developer.gosub.io/)
* [Wiki](https://wiki.developer.gosub.io/)

## Contents
* [Introduction to the Makefiles](#introduction-to-the-makefiles)
    * [Building the Entire Project](#building-the-entire-project)
    * [Running Tests and Formatter](#running-tests-and-formatter)
    * [Fix Formatting](#fix-formatting)
    * [Running Benchmarks](#running-benchmarks)
* [Code Style](#code-style)
* [Doc Comments](#doc-comments)
* [Modules](#modules)
* [Signed Commits](#signed-commits)
* [PR Guidelines](#pr-guidelines)
* [What to do?](#what-to-do)
    * [Your first contribution](#your-first-contribution)
    * [Other paths](#other-paths)

## Introduction to the Makefiles
Before writing any code, it's good to get familiar with our Makefile(s). Use "make" or "make help" for a list of available commands. 

### Building the Entire Project
In the root directory, run `make build` to build the entire project. If you're only interested in building components of the engine (e.g., the core engine or the bindings) then you can use `cargo build` in the relevant directory.

Note that a full build includes the GTK4 and Cairo render backends, which need OS packages
installed first — see [`docs/examples.md`](docs/examples.md) for the list. If you are working on
one of the parsers you do not need any of that, and can stay on a much faster loop:

```bash
cargo test -p gosub_css3 -p gosub_html5    # ~470 tests, no system dependencies
```

### Running Tests and Formatter
In the root directory, run `make test` to run all unit tests and the format checker. If there are issues with formatting, you should see a `git diff`-like output in the terminal. Please resolve these formatting issues prior to pushing any code! This is validated in our CI when creating a pull request.

### Fix Formatting
In the root directory, run `make format` to have clippy automatically fix some of the formatting issues.

### Running Benchmarks
In the root directory, run `make bench` to run the benchmarks.


## Code Style
We use cargo's built-in formatter. When running `make test`, the formatter will also run (after all the tests) and display any issues in a `git diff`-like output in the terminal. Please resolve these formatting issues prior to pushing any code! This is validated in our CI when creating a pull request.

## Doc Comments
On structs/struct properties, enums and methods we typically add doc comments notated by a triple forward slash `///`. These comments are used while generating the Gosub API docs after merging into main. For "private" or internal comments, you can stick to the double forward slash `//`.

## Modules
When it comes to Rust modules, there are typically two approaches:
1. using `mod.rs`
2. using `module_name.rs` and optionally a `module_name/` directory (if necessary)

Gosub has adopted the second (2) option to handle modules.

If you are introducing a new module, `mymodule`, create a `mymodule.rs` in the `src/` directory and add it to `lib.rs`.

If your module requires submodules, create a `mymodule/` directory in the `src/` directory with `submodule1.rs`, `submodule2.rs`, etc. and add declare the modules in `mymodule.rs`.

The structure should look something like:

```text
src/
    lib.rs
    mymodule.rs
    mymodule/
        submodule1.rs
        submodule2.rs
```

lib.rs:
```rust
pub mod mymodule;
```

mymodule.rs:
```rust
pub mod submodule1;
pub mod submodule2;
```

## Signed Commits
We require signed commits on *every* commit prior to merging into main. Signed commits can be tricky to setup so please follow [this guide](https://docs.github.com/en/authentication/managing-commit-signature-verification/signing-commits) as it goes into more detail than I can go into here.

## PR Guidelines
When creating PRs, please keep the following things in mind:
* Try to keep the content of your PR short-and-sweet
    * It makes things a lot harder when a PR with 10,000 line changes is dropped. We are a small team and building a browser is time consuming; having shorter PRs can reduce the delay in merging your code (which is potentially blocking other PRs.)
    * It also reduces risk of error if the PR contains less changes - it may be hard to capture edge cases when reviewing 10, 20, 30+ files
* Add a quick summary describing what your PR does, its intentions and/or any other useful bits of information.
    * This will give the reviewers more context into your changes. Keep in mind not everyone (or no one) is an expert on the entire codebase - the extra context can help understand the decisions made within the PR.
* Try to keep commit count between 1-3 per PR
    * We prefer small commit counts (ideally 1 commit per PR but sometimes this can be tricky...) to avoid muddying up the history and help with `git bisect` to narrow down a PR that possibly introduced any regressions.
    * It's a common practice for us to rebase & squash commits before/during the PR creation. Some of us work on multiple computers and can rack up 10+ commits - these are squashed to (ideally) 1 prior to merging into main. Your PR might not be approved until you reduce your commit count.
* We are not Rust experts
    * Most of the core developers are *relatively* new to Rust but have good experience in other languages. We may not know every idiomatic Rust way but we are trying to move in that direction as we collectively gain experience in the langauge.

## What to do?
Great! You're now familiar with our contribution guidelines and maybe even a bit of our codebase. So...now what?

### Your first contribution

If you want something concrete to work on rather than a research topic, the shortest path in is
a failing web-platform-test. WPT is the browser industry's shared conformance suite, so each
failing subtest is a specific, already-triaged statement about something the engine gets wrong —
no need to invent a task or guess whether it is wanted.

**[`docs/wpt-quickstart.md`](docs/wpt-quickstart.md) takes you from cloning this repo to a fixed
test in about ten minutes** — every command, nothing to configure, no system packages. Start
there. [`docs/wpt.md`](docs/wpt.md) is the full reference behind it.

The short version: point `$WPT_ROOT` at a wpt checkout, then

```bash
cargo run --release -p gosub-wpt -- "$WPT_ROOT" css/css-values
```

prints a pass rate per directory and a list of suites. Pick a file that is **partly** passing —
those are ones where the engine already understands the shape of the thing and is wrong about a
detail, which is an afternoon's work rather than a project. Run that one file to see the failing
subtests by name, fix it in engine code, and regenerate the baseline.

`tests/wpt/expectations-css.txt` is the current baseline for the CSS parser and holds a few
thousand of these. A `CRASH` line in it is the best thing to pick up: it means engine code
panicked on input a real page could carry.

One rule: engine *behaviour* is fixed in engine code. `gosub_domjs` is a binding layer, so
adding or correcting a Web API that a test needs belongs there — but a shim that hard-codes an
answer, rather than reading it out of the engine, makes the test green and tells us nothing.

### Other paths

Beyond that, there are a few main directions:
* Research
    * Research? That doesn't sound like code... and you're right! None of us have built a browser before and we are learning (a lot) as we go. Researching different components (rendering, tokenizing, parsing, etc.) and starting/participating in discussions is a valuable contribution even when you are not adding lines of code to the project.
* Study the codebase
    * If you're not exactly sure what to do, it might be a good opportunity to spend some time sifting through the codebase and understanding how things are structured.
* Issue tackling
    * We don't have a high volume of issues at the moment, but there may be some that a new contributor can pick up!
* Specification compliance
    * We are not completely in compliance with certain specifications (CSS, DOM) and could likely use some help there.
* Write tests
    * We are not crazy about hitting 100% code coverage, but there may be cases that make sense to test that we haven't caught yet. If you see any opportunities, please feel free to add more tests!
* Make the code more "Rust"-y
    * As mentioned, we are not Rust experts. If you are more seasoned in the language, you can help make our code more idiomatic.
* Fill in the missing gaps
    * This will take more familiarity with the codebase, but once you have a good grasp, you can help fill in the missing gaps - helping implement javascript, engine API for user agents, DOM/CSSOM, render tree, etc.
* Ask us
    * Completely lost on what to work on? Ask in [our developer chat](https://chat.developer.gosub.io/) what we need help with currently - we might have something in mind!
