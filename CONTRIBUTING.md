# Contributing to Gosub
Welcome and thanks for your interest in contributing to Gosub!

This is an initial (but not fully complete) contribution guide.

**Useful Links for Developers:**
* [Developer Chat](https://chat.developer.gosub.io/)
* [Wiki](https://wiki.developer.gosub.io/)
* [API Docs](https://docs.developer.gosub.io/)
* [Benchmarks](https://bench.developer.gosub.io/)

## Contents
* [Introduction to the Makefiles](#introduction-to-the-makefiles)
    * [Building the Entire Project](#building-the-entire-project)
    * [Running Tests and Formatter](#running-tests-and-formatter)
    * [Fix Formatting](#fix-formatting)
    * [Running Benchmarks](#running-benchmarks)
    * [Building C API Bindings](#building-c-api-bindings)
* [Code Style](#code-style)
* [Doc Comments](#doc-comments)
* [Modules](#modules)
* [Signed Commits](#signed-commits)
* [PR Guidelines](#pr-guidelines)
* [What to do?](#what-to-do)

## Introduction to the Makefiles
Before writing any code, it's good to get familiar with our Makefile(s). Use "make" or "make help" for a list of available commands. 

### Building the Entire Project
In the root directory, run `make build` to build the entire project. If you're only interested in building components of the engine (e.g., the core engine or the bindings) then you can use `cargo build` in the relevant directory.

### Running Tests and Formatter
In the root directory, run `make test` to run all unit tests and the format checker. If there are issues with formatting, you should see a `git diff`-like output in the terminal. Please resolve these formatting issues prior to pushing any code! This is validated in our CI when creating a pull request.

### Fix Formatting
In the root directory, run `make format` to have clippy automatically fix some of the formatting issues.

### Running Benchmarks
In the root directory, run `make bench` to run the benchmarks.

### Building C API Bindings
In the `crates/gosub_bindings` directory, run `make bindings` to build the C static libraries. For more information on this, see the [README](crates/gosub-bindings/README.md).

Can also run `make test` in the same directory to build and run the tests for the C bindings.

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

At this point, there are a few main paths:
* Research
    * Research? That doesn't sound like code... and you're right! None of us have built a browser before and we are learning (a lot) as we go. Researching different components (rendering, tokenizing, parsing, etc.) and starting/participating in discussions is a valuable contribution even when you are not adding lines of code to the project.
* Study the codebase
    * If you're not exactly sure what to do, it might be a good opportunity to spend some time sifting through the codebase and understanding how things are structured.
* Issue tackling
    * We don't have a high volume of issues at the moment, but there may be some that a new contributer can pick up!
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
