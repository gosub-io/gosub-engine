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
| CSS parser component - `css/css-syntax`, `css/css-values` | 309 files, 5314 subtests | 307 (5.8%) | 30s |
| nightly - every testharness suite | 27,301 files | ~2% | 150s |
| reftests - `css/CSS2` | 5,952 | ~1560 (26%) | 455s |

The CSS component's 5.8% is close to a floor rather than a measurement of the parser: 156 of
its 309 suites need `getComputedStyle`, which does not exist, and most of the rest assert a
canonical serialization the engine does not produce. Where the parser is actually reached the
numbers are much higher - `calc-size` at 71%, `urls` at 31%, `position` at 25%.

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

### Running one component

A directory argument runs every testharness suite underneath it, so a component can be named
rather than listed. This is the way to see where one part of the engine stands:

```bash
cargo run --release -p gosub-wpt -- "$WPT_ROOT" css/css-values
```

Discovery selects on the harness script rather than on the path, so the reftest halves, the
`conformance-checkers/` fixtures and the manual tests in the tree are left out - `css/css-values`
is 270 suites, not the 518 `.html` files it contains.

The run ends with a rollup per directory and the totals:

```
  css/css-values                 █░░░░░░░░░  192/4516    4.3%
  css/css-values/calc-size       ███████░░░     5/7     71.4%
  css/css-values/urls            ███░░░░░░░    39/126   31.0%

  309 files: 16 fully passing, 285 with failures, 5 could not run
  5314 subtests: 307 passed, 5007 failed
  3 files crashed the engine (grep the run for CRASH)
```

Grouping is by each suite's own directory, not by a fixed prefix depth, because that is the
granularity work gets picked at: `calc-size` at 71% and `calc-size/animation` at 0% is the
useful shape, and one averaged line is not.

### Picking something to fix

The tractable work is in the files that are **partly** passing. A suite at 0/40 is usually
missing a whole binding - `getComputedStyle`, the CSSOM stylesheet - and is a project rather
than an afternoon. A suite at 6/24 has an engine that already understands the shape of the
thing and is wrong about a detail, which is what you want.

```bash
# Suites with both passes and failures - the shortlist.
cargo run --release -p gosub-wpt -- "$WPT_ROOT" css/css-values 2>/dev/null \
    | grep -aE '[0-9]+ passed, [0-9]+ failed$' | grep -av ': 0 passed' | grep -av ' 0 failed'
```

A worked example, start to finish.

**1. Find one.** `css/css-values/viewport-units-parsing.html: 6 passed, 18 failed`.

**2. Read the failures.** Run the single file; every failing line carries the assertion and
what the engine gave instead. Add `-v` to see the passing subtests too.

```
$ cargo run --release -p gosub-wpt -- "$WPT_ROOT" css/css-values/viewport-units-parsing.html
  FAIL e.style['width'] = "1svw" should set the property value - assert_not_equals: property should be set got disallowed value ""
  FAIL e.style['width'] = "1lvw" should set the property value - assert_not_equals: property should be set got disallowed value ""
  FAIL e.style['width'] = "1dvw" should set the property value - assert_not_equals: property should be set got disallowed value ""
```

`vw` passes and `svw`/`lvw`/`dvw` do not, so this is not "viewport units are missing" - it is
one list somewhere that is short of the small-, large- and dynamic-viewport spellings.

**3. Find the engine code.** Grep for a value that *does* work, next to one that does not:

```bash
rg '"vw"' crates/gosub_css3/src/
```

`crates/gosub_css3/src/matcher/syntax_matcher.rs` has `LENGTH_UNITS`, which lists `vh vw vmax
vmin vb vi` and none of the prefixed forms - while `stylesheet.rs` already resolves `svw`,
`lvw` and `dvw` to pixels. The validator and the resolver disagree, and the validator is the
one that is wrong.

**4. Fix it in engine code**, never in the bindings (see [The one rule](#the-one-rule)). Here
that is 18 strings added to `LENGTH_UNITS`, which takes the file to 24/24.

**5. Check the baseline.** The run now fails, and that is correct:

```
$ cargo run --release -p gosub-wpt -- "$WPT_ROOT" --all --expect tests/wpt/expectations-css.txt
  UNEXPECTED PASS e.style['width'] = "1svw" should set the property value
  ...
  309 files: 17 fully passing, 284 with failures, 5 could not run
  5314 subtests: 325 passed, 4989 failed
```

An UNEXPECTED PASS is a listed failure that started working. It fails the run on purpose, so
that improving behaviour forces the baseline to be regenerated and the file always says what
the engine actually does.

**6. Regenerate, and commit the diff alongside the fix:**

```bash
cargo run --release -p gosub-wpt -- "$WPT_ROOT" css/css-syntax css/css-values \
    --write-expectations > tests/wpt/expectations-css.txt
```

A `CRASH` line is the best thing to pick up of all: it means engine code panicked on input a
real page could carry, which is a bug of a different order from a missing feature.

### The expectations file

`tests/wpt/expectations.txt` is the committed baseline: which suites are covered, and
which subtests are known to fail. Five record types - `FILE`, `FAIL <path> :: <name>`,
`HARNESS` (the harness itself did not finish cleanly), `ERROR` (the suite cannot run at
all, usually a support file outside the sparse checkout) and `CRASH` (the engine panicked).

`CRASH` is kept apart from `ERROR` on purpose. An `ERROR` is this tool's limitation; a `CRASH`
is a panic in engine code that a real page could reach, and folding the two together would let
the more serious one settle into the baseline unnoticed. The runner catches the panic so one
bad input cannot end a corpus run at whichever suite reaches it first.

Control characters in subtest names are escaped - `\n`, `\r`, `\t`, and everything else in the
C0 range as `\xNN`. `css/css-syntax` walks that whole range looking for what a parser must
treat as whitespace, and writing those bytes through left a baseline that `grep` and `git diff`
both refused to treat as text.

`tests/wpt/expectations-css.txt` is the same format for the CSS parser component
(`css/css-syntax` and `css/css-values`). It is **not** gated in CI: it exists so the parser's
progress is measurable and so a contributor can pick a failing subtest and go fix it.

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

`element.style`: the specified-value half of `CSSStyleDeclaration` - `getPropertyValue`,
`setProperty`, `removeProperty`, `cssText`, `length`, `item`, and named access
(`style.fontSize`, `style['font-size']`) through a proxy that maps the IDL spelling back to
the CSS one. Assigning to `style` itself forwards to `cssText`, as `[PutForwards=cssText]`
requires.

The block **is** the element's `style` attribute, and whether a value is accepted at all is
decided by `gosub_css3`: the declaration goes through the real parser and is then checked
against the property's syntax definition. So a green `test_valid_value` says the CSS parser
took the value, and a green `test_invalid_value` says it refused one it should refuse.

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
- **No CSSOM serialization.** `getPropertyValue` gives back the text the author wrote, because
  `CssValue`'s `Display` is a debug rendering rather than a CSS serializer - a `List` prints as
  `List(a, b, c)`. Every suite asserting a canonical form therefore fails: `calc()`
  normalization (`calc(1vh + 2px + 3%)` should serialize as `calc(3% + 2px + 1vh)`) is a few
  hundred subtests on its own. Writing a serializer in the bindings would make those tests
  measure the binding rather than the engine, so the work belongs in `gosub_css3`.
- **No `getComputedStyle`**, so nothing about the cascade, inheritance or used values is
  reachable. 156 of the 309 suites in the CSS component need it and none of them can pass
  without it - it is the single largest thing standing between the engine and those numbers.
- **No CSSOM stylesheet.** `style.sheet`, `insertRule`, `deleteRule` and `cssRules[i].cssText`
  are all missing, which is what `test_valid_selector` and `test_valid_rule` drive - so the
  selector and at-rule parsers have no coverage here yet.
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
