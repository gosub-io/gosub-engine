# WPT quickstart: from clone to a fixed test

Fix your first bit of the engine in about ten minutes. No prior knowledge of the codebase,
no system packages, nothing to configure.

[WPT](https://github.com/web-platform-tests/wpt) is the conformance suite the browser vendors
share. Every failing test in it is a specific, already-agreed statement about something the
engine gets wrong — so you never have to invent a task or guess whether the work is wanted.

## All of it, in one block

```bash
# 1. The engine.
git clone https://github.com/gosub-io/gosub-engine.git
cd gosub-engine

# 2. A wpt checkout, pinned to the commit this repo is measured against.
#    Blob-less and sparse: a few hundred MB, not several GB. `wpt/` is gitignored.
git clone --filter=blob:none --sparse \
    https://github.com/web-platform-tests/wpt.git wpt
git -C wpt sparse-checkout set \
    resources common css/css-syntax css/css-values css/support css/reference
git -C wpt checkout "$(cat tests/wpt/wpt-commit.txt)"
export WPT_ROOT="$PWD/wpt"

# 3. Run a component. First build takes about a minute; the run takes about 30 seconds.
cargo run --release -p gosub-wpt -- "$WPT_ROOT" css/css-values
```

That is the whole setup. If the last command printed a table of directories with bars next to
them, you are ready to fix something.

## What you just ran

`gosub-wpt` parses each test with the real `gosub_html5`, runs its scripts against the real
`gosub_css3` through a small JavaScript engine, and reports what the test's own assertions say.
It is not a browser: there is no navigation, no network, and no window.

The run ends like this:

```
  css/css-values                 █░░░░░░░░░  192/4516    4.3%
  css/css-values/calc-size       ███░░░░░░░    38/120   31.7%
  css/css-values/urls            ███░░░░░░░    39/126   31.0%

  270 files: 15 fully passing, 250 with failures, 5 could not run
  5030 subtests: 328 passed, 4702 failed
```

Low numbers are expected and are not the interesting part. What matters is which suites are
*partly* passing.

The numbers in this document move as the engine improves — including because of what you are
about to do to them. Treat them as the shape of the exercise, not as something to match.

## Find something to fix

This command is the live part of this document — it asks the engine, right now, what is worth
picking up:

```bash
cargo run --release -p gosub-wpt -- "$WPT_ROOT" css/css-values --shortlist
```

```
  Partly passing, nearest to working first:
     94.3%  css/css-values/progress-invalid.html            33/35, 2 left
     71.4%  css/css-values/random-item-invalid.html         10/14, 4 left
     53.2%  css/css-values/tree-counting/calc-sibling...    25/47, 22 left
```

Take one off the top. A partly-passing suite means the engine already understands the shape of
the thing and is wrong about a detail — that is an afternoon. Suites that fully fail are left
out, because those are usually missing a whole feature and are a project rather than a first
task.

If the listing opens with a **Crashes** section, start there instead: those are suites that
panicked the engine, which is a bug a real page could reach rather than a feature it lacks.
There are none at the time of writing, but they come back.

If nothing there appeals, the two big structural gaps are always open, and either is worth
hundreds of subtests: **CSSOM serialization** (the engine hands back the author's text rather
than a canonical form) and **`getComputedStyle`** (156 of the 309 CSS suites need it and none
can pass without it).

Run your pick on its own. Every failing line carries the assertion and what the engine gave
instead:

```bash
cargo run --release -p gosub-wpt -- "$WPT_ROOT" <the-file-you-picked>
```

## A worked example

> This walks a real bug, as it stood in September 2026. **It may already be fixed** — it is a
> small one, and this page exists to get it fixed. If the shortlist above handed you something
> else, follow along anyway: the method is the same every time, and the point is the shape of
> the loop rather than this particular list of units.

The file was `css/css-values/viewport-units-parsing.html`, at 9 passed, 15 failed:

```
  FAIL e.style['width'] = "1svh" should set the property value - assert_not_equals: property should be set got disallowed value ""
  FAIL e.style['width'] = "1lvmax" should set the property value - assert_not_equals: property should be set got disallowed value ""
  ...
```

**Read what passes against what fails.** `1svw` was accepted and `1svh` was not. So this was not
"viewport units are missing" — it was one list somewhere that had some spellings and not others.
That reading is the whole trick, and it works on almost every one of these.

**Then grep for a value that works, next to one that does not:**

```bash
rg '"svw"' crates/gosub_css3/src/
```

That landed on `LENGTH_UNITS` in `crates/gosub_css3/src/matcher/syntax_matcher.rs`, which listed
`svw`, `lvw` and `dvw` but none of the `h` / `i` / `b` / `min` / `max` spellings that go with
them — while `stylesheet.rs` already knew how to resolve them. The validator and the resolver
disagreed, and the validator was the one that was wrong. The fix was the missing spellings, and
it took the file to 24/24.

**Fix it in engine code, never in the test bindings.** `crates/gosub_domjs` exists only to let
these tests reach the engine; a shim there that makes a test go green tells us nothing. See
[The one rule](wpt.md#the-one-rule).

## Prove it

Run your file again — it should be clean. Then check the whole component against the committed
baseline:

```bash
cargo run --release -p gosub-wpt -- "$WPT_ROOT" --all --expect tests/wpt/expectations-css.txt
```

This now **fails**, and that is correct — you will see an `UNEXPECTED PASS` line for each
subtest you fixed.

An UNEXPECTED PASS is a subtest that started passing and is not yet in the baseline (which
records what passes). It fails the run on purpose, so
that improving the engine forces the baseline to be updated and the file always says what the
engine actually does. Regenerate it:

```bash
cargo run --release -p gosub-wpt -- "$WPT_ROOT" css/css-syntax css/css-values \
    --write-expectations > tests/wpt/expectations-css.txt
```

Run it once more; it should pass now. Commit the engine fix and the regenerated baseline
together — the diff in the baseline is the evidence that the fix did something.

## Before you open the PR

```bash
make test          # fmt, clippy, smoke, unit tests
```

Commits must be signed, and PRs are easiest to review when they are small — see
[CONTRIBUTING.md](../CONTRIBUTING.md).

## Where to go next

- [`docs/wpt.md`](wpt.md) — everything about the harness: what is and is not bound, how a test
  runs, the expectations format, the reftest runner, and what CI does.
- [`docs/css.md`](css.md) — `gosub_css3` from text to computed value, if you want to stay in
  the CSS parser.
- [`docs/crates.md`](crates.md) — where everything else lives.

Then run the shortlist again and take the next one.
