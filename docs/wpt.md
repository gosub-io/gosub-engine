# Running web-platform-tests

Two harnesses, because wpt holds two kinds of test that are checked in completely
different ways. Neither is a browser: both drive the engine directly.

| | testharness.js | reftests |
|---|---|---|
| How a test passes | JS assertions report themselves | two renders are pixel-identical |
| What it needs | a DOM and a JS engine | layout, painting, fonts |
| Tool | `bin/gosub-wpt` (QuickJS via rquickjs) | `scripts/wpt-reftest.py` -> `gosub-screenshot` |
| In CI | the `wpt` gate on every PR, plus a nightly | manual only |

## What of wpt we can actually run

Of ~57k `.html` files in the corpus (excluding `-ref`/`-notref`, which are the reference
halves of reftests rather than tests):

- **~27,300** load `resources/testharness.js` - the testharness harness can run these.
- **~19,800** are reftests - the reftest runner can run these.
- The rest are manual tests and conformance-checker fixtures. Neither harness can do
  anything with them: they parse, report zero subtests, and cost runtime. The nightly
  filters them out with `grep -lFr 'resources/testharness.js'`.

Nothing here runs `.any.js` / `.window.js` / `.worker.js` wrappers, iframes, workers, or
anything needing navigation or a network.

## Where the numbers stand

Measured at the pinned commit; regenerate rather than trust these.

| Run | Tests | Passing | Time |
|---|---:|---:|---:|
| `wpt` gate - `dom/events`, `html/dom` | 621 files (616 report), 1132 subtests | 101 (8.9%) | 8s |
| nightly - every testharness suite | 27,301 files | ~2% | 150s |
| reftests - `css/CSS2` | 5,952 | ~1560 (26%) | 455s |

The reftest rate is the higher one because those exercise layout and painting, which the
engine does, rather than DOM and Web APIs, which it mostly does not. Within CSS2 the
spread is the interesting part: `fonts`, `syntax`, `borders` and `normal-flow` sit near
40-50%, while `floats-clear`, `tables`, `linebox` and `css1` are all under 6%.

**Reftest results are not yet stable run to run.** Two full CSS2 runs on one machine gave
1558 and 1597, with 133 tests changing status in both directions, concentrated in
`backgrounds`. A third on another machine gave 1562. The error and skip sets are identical
across all of them, so discovery is deterministic and only the pixel comparison moves.
That is why the reftests are manual and ungated: a committed baseline would go red on
noise. `--settle` was tried against the theory that images had not finished decoding; it
made things worse, so the cause is still open.

## The CI jobs

- **`wpt`** (`ci.yaml`, every push and PR) - runs the gate against
  `tests/wpt/expectations.txt` at the commit in `tests/wpt/wpt-commit.txt`, and fails if
  the results move in either direction. Also uploads a coverage report. Without `WPT_ROOT`
  the test skips, so an ordinary `cargo test` needs no checkout.
- **`wpt-full`** (`nightly.yaml`, 02:00 UTC) - every testharness suite at wpt HEAD, no
  baseline, report as an artifact. A failing subtest cannot turn it red.
- **`wpt-reftests`** (`wpt-reftests.yaml`, manual) - takes a subtree and a settle value as
  inputs. Not scheduled: a run needs a cairo `gosub-screenshot` build the other jobs'
  caches cannot share, and nothing it produces is gated.

## Running the reftests

```bash
# Ahem ships inside wpt; without it fontconfig substitutes and nearly everything fails
# on sub-pixel differences.
mkdir -p ~/.local/share/fonts && cp <wpt>/fonts/Ahem.ttf ~/.local/share/fonts/ && fc-cache -f
fc-match Ahem      # must say Ahem.ttf

cargo build --release -p gosub-screenshot --no-default-features --features backend_cairo
python3 scripts/wpt-reftest.py --wpt-root <wpt> --out /tmp/reftest --report css/CSS2
```

The sparse checkout needs `resources fonts css/support css/reference` plus the subtree -
`css/support` and `css/reference` hold the reference pages. `--report` writes
`failures.html` with test, reference and diff side by side; `--chrome` adds a headless
Chromium render of each failure next to them. `scripts/wpt-fonts.conf` pins the generic
families so results do not depend on the distro's fontconfig defaults.

## The testharness harness

The engine has no scripting environment yet, so the WPT `testharness.js` suites cannot run
against it as they stand. `gosub_domjs` is a stopgap: a **test-only** DOM binding over a
small JavaScript engine (QuickJS, through `rquickjs`), enough to let those tests drive the
engine's own DOM.

CI covers `dom/events` and `html/dom` — the two directories these bindings actually reach.
The harness itself is directory-agnostic: point it at any tree of `testharness.js` files.
Form controls are **not** covered here. That work lives on its own branch and needs engine
modules (`edit`, `form`, `focus`) that are not on main yet.

It exists to find bugs, not to run websites.

### Setup

The checkout is pinned: `tests/wpt/wpt-commit.txt` holds the commit CI uses, and results are
only comparable against that one.

```bash
git clone --filter=blob:none --sparse https://github.com/web-platform-tests/wpt.git
cd wpt
git sparse-checkout set resources common dom/nodes dom/events html/dom
git checkout "$(cat …/tests/wpt/wpt-commit.txt)"
```

```bash
cargo run -p gosub-wpt -- <wpt-root> <test.html>... [-v]
```

Paths are taken relative to the wpt root when they are not found as given. The exit code is
non-zero if any subtest failed.

### The expectations file

`tests/wpt/expectations.txt` is the committed baseline: which suites are covered, and
which subtests are known to fail. Four record types - `FILE`, `FAIL <path> :: <name>`,
`HARNESS` (the harness itself did not finish cleanly) and `ERROR` (the suite cannot run at
all, usually a support file outside the sparse checkout).

Files are listed explicitly rather than globbed, so adding tests to a wpt checkout cannot
silently change what is covered.

```bash
cargo run -p gosub-wpt -- <wpt-root> --all --expect tests/wpt/expectations.txt
```

That is what `cargo test -p gosub-wpt --test wpt_conformance` runs when `WPT_ROOT` is set,
and what the `wpt` CI job runs at the pinned commit. Without `WPT_ROOT` the test skips,
so an ordinary `cargo test` needs no checkout.

### Running a list of tests

`--tests-from <file>` takes the paths from a file, one per line (`-` reads stdin); blank
lines and `#` comments are skipped. It is the only way to run a corpus of any size: the
whole of wpt is ~57k `testharness.js`-eligible files, which is well past `ARG_MAX`, and
batching the run with `xargs` to get under the limit would write a separate `--report` per
batch instead of one page for the run.

```bash
cd "$WPT_ROOT" && find . -name '*.html' | sed 's|^\./||' \
    | grep -vE '(-ref|-notref)\.html$' | sort > /tmp/all.txt
cargo run --release -p gosub-wpt -- "$WPT_ROOT" --tests-from /tmp/all.txt --report all.html
```

Two thirds of those files are not testharness suites at all - reftests, the
`conformance-checkers/` fixtures, manual tests - and report zero subtests. Filtering them
out first is worth it for both the runtime and the readability of the page.

### The overview page

`--report page.html` writes a coverage-report view of the whole run: the headline rate, then
every directory with its pass/fail split and a bar, expandable to the suites underneath.
Suites that could not run at all, or whose harness did not finish cleanly, carry a badge.

```bash
cargo run --release -p gosub-wpt -- "$WPT_ROOT" --all \
    --expect tests/wpt/expectations.txt --report wpt-report.html
```

Rates are subtests, not files, and known failures count as failures - the page shows the
corpus as it is, not as the expectations describe it. The template is
`bin/gosub-wpt/report.html`; the run inlines its data, the wpt commit and the date.

A listed test that starts passing is an **UNEXPECTED PASS** and fails the run. That is
deliberate: improving behaviour is supposed to make you regenerate the baseline and commit
the diff, so the file always says what the engine actually does.

```bash
cargo run --release -p gosub-wpt -- "$WPT_ROOT" --write-expectations $(paths...) \
    > tests/wpt/expectations.txt
```

Diagnostics (console output, listener and timer exceptions, scripts that threw) go to
stderr; only results go to stdout, so regenerating never picks up stray lines.

### The one rule

**The bindings hold no DOM logic.** Every property reads or writes the real document, so a
passing test says something about the engine rather than about the binding layer. When a
test needs behaviour the engine does not have, the fix belongs in engine code — never in a
shim that makes the test go green.

### How a test runs

1. The file is parsed into a `DocumentImpl` by `gosub_html5`.
2. A fresh QuickJS context gets `self`, then wpt's own `testharness.js`.
3. `document` is installed **after** testharness.js. testharness picks its environment by
   looking for `document` on the global scope; without one it uses the shell environment,
   which needs no window, no load event and no result-reporting DOM. Installing `document`
   afterwards keeps it in that mode while still giving tests a DOM.
4. Every `<script>` in the document runs in tree order (`testharness.js` and
   `testharnessreport.js` are skipped — the driver loads the first and replaces the second).
   Microtasks are drained after each one.
5. The driver calls `done()`, then pumps the timer queue until the harness reports or the
   queue runs dry.
6. If the queue drains and nothing has reported, the driver calls testharness's `timeout()`.
   The shell environment has no default timeout, so an async test whose event never arrives
   would otherwise hang the run forever; this turns it into a TIMEOUT result instead.
7. Results come out of an `add_completion_callback` hook.

### Timers

There is no clock. `setTimeout`, `setInterval`, `requestAnimationFrame` and their cancel
functions all feed one queue ordered by due time and then insertion order, and firing a
callback advances a virtual "now" to that callback's due time. Nothing waits on wall-clock
time, and a test that schedules a 10-second timeout costs nothing to run.

`requestAnimationFrame` resolves one frame (16ms of virtual time) later and passes a
timestamp. Nothing paints — it is the delay that matters, since 57 of the forms tests use
rAF purely to wait a turn.

testharness passes `null` where a delay or a timer id is expected, which is not the same as
omitting the argument, so both are taken as raw values and coerced.

### Events

`addEventListener`/`removeEventListener`/`dispatchEvent` are on nodes, on `document` and on
the global object; `Event` is constructible with `bubbles`/`cancelable`/`composed`. Dispatch
implements capture → at-target → bubble over the **real document tree**, with
`stopPropagation`, `stopImmediatePropagation`, `preventDefault`, the `once` and `capture`
listener options, and the spec's dedup rule (same type + callback + capture is ignored).

`element.click()` fires a click event but has **no activation behaviour**: a checkbox does
not toggle and a submit button does not submit, because that lives in `gosub_engine`'s
private `edit`/`form` modules. Tests that click and then wait for the resulting change now
report TIMEOUT rather than hanging.

Removed listeners are tombstoned rather than deleted, because dispatch holds indices into
the listener list and has to observe removals made by listeners that run before them.

### What is bound

`document`: `getElementById`, `createElement`, `createTextNode`, `querySelector`,
`getElementsByTagName`, `body`, `head`, `documentElement`.

`Node` also carries `addEventListener`, `removeEventListener`, `dispatchEvent` and `click`.

`Node`: `nodeType`, `nodeName`, `tagName`, `localName`, `parentNode`, `parentElement`,
`childNodes`, `children`, `firstChild`, `appendChild`, `removeChild`, `remove`,
`hasChildNodes`, `get`/`set`/`remove`/`hasAttribute`, `getAttributeNS`/`setAttributeNS`,
`id`, `className`, `textContent`, `outerHTML`, `querySelector`, `getElementsByTagName`, and
the option/textarea reflections `value`, `label`, `text`, `type`.

Node wrappers are cached per node, so `a.parentNode === b` holds.

### What is not

- **No activation behaviour** behind `click()`, and no `focus()`/`blur()`/`activeElement`
  (288 uses in the forms corpus) — both need engine code that is not public yet.
- **No `CustomEvent`, `MouseEvent` or `KeyboardEvent`** constructors, and no `EventTarget`
  constructor. The forms corpus never uses the first; it uses the mouse and keyboard ones in
  13 files.
- **No interface hierarchy.** One `Node` class dispatches on tag name, so `instanceof`,
  `Option`, `NodeList` and prototype-chain tests fail.
- **No layout and no navigation**, so iframes, `getBoundingClientRect` and form submission
  are out of reach.
- **Scripts run after parsing**, not during it, so document.write and parser-timing tests
  are meaningless here.
- `querySelector` handles a single compound selector (`tag`, `#id`, `.class` and
  combinations) and throws on anything else, rather than silently mismatching.
- The document has no attribute namespaces; `setAttributeNS` parks the value under a key no
  HTML attribute name can produce, which keeps it out of the reflection path.
- `appendChild` cannot throw `HierarchyRequestError` properly — the document refuses to
  build a cycle instead of raising, so the binding turns that refusal into a plain error.
