# Component tool reference

These binaries each exercise a single crate in isolation — the HTML5 parser, CSS3 parser,
config store, etc. They are useful for development and debugging but are not the primary way to
drive the engine.

To see the full `GosubEngine` stack in action (multi-zone/tab model, async networking, event
bus), run the engine examples instead:

```bash
cargo run --example hello-world    # single tab, headless
cargo run --example multi-tab      # 25 tabs, live progress bars
cargo run --example gtk-cairo      # GTK4 window
cargo run --example egui-vello     # egui/wgpu window
```

See [`examples/README.md`](../examples/README.md) for details.

---

## config-store

View and modify the configuration store.

```bash
$ cargo run -r --bin config-store list

dns.cache.max_entries                   : u:1000
dns.cache.ttl.override.enabled          : b:false
dns.local.enabled                       : b:true
useragent.default_page                  : s:about:blank
useragent.tab.max_opened                : i:-1
...

$ cargo run -r --bin config-store search --key 'user*'

useragent.default_page                  : s:about:blank
useragent.tab.close_button              : m: left
useragent.tab.max_opened                : i:-1
```


## css3-parser

Parse a CSS stylesheet and print the parse tree (or any errors encountered). Does not validate
property value syntax — `color: 1%` will parse without error.

```bash
$ cargo run -r --bin css3-parser file://tests/data/css3-data/test.css

Running css3 parser of (54.00 B) took 0 ms.
[Stylesheet (1 rules)]
  [Rule]
    [SelectorList (2)]
      [Selector]
        [Type] div
      [Selector]
        [Type] a
    [Block (2 declarations)]
      [Declaration] color
        String("white")
      [Declaration] border
        List([Unit(1.0, "px"), String("solid"), String("black")])
```


## gosub-parser

Fetch a URL, parse the HTML5 and any linked CSS, then print parse errors and timing statistics.

```bash
$ cargo run -r --bin gosub-parser https://news.ycombinator.com

Parsing url: Url { scheme: "https", ... host: Some(Domain("news.ycombinator.com")), ... }

Found 1 stylesheets
Stylesheet location: "https://news.ycombinator.com/news.css?..."

Parse Error: expected-doctype-but-got-start-tag
Parse Error: link element with rel attribute 'icon' is not supported in the body
...

Namespace            |    Count |      Total |        Min |        Max |        Avg
------------------------------------------------------------------------------------
html5.parse          |        1 |      605ms |      605ms |      605ms |      605ms
css3.parse           |        1 |      613µs |      613µs |      613µs |      613µs
```


## display-text-tree

Fetch a URL and print a plain-text representation — all text nodes from the parsed document,
with no layout or styling applied. Useful for a quick sanity check on what the parser sees.

```bash
$ cargo run -r --bin display-text-tree https://gosub.io
```


## html5-parser-test

Run the html5lib tree-builder test suite from the command line. The test data files must be
reachable from the working directory; run from the repo root.

```bash
$ cargo run -r --bin html5-parser-test
```


## parser-test

A focused parser development harness for running specific HTML5 parser tests during development.
Intended for use while actively working on the parser; not a substitute for the full test suite.

```bash
$ cargo run -r --bin parser-test
```


## run-js

Run a JavaScript file through the V8 engine. There is no DOM or Web API binding, so browser
globals (`console`, `document`, `fetch`, etc.) are not available.

```javascript
var a = 1 + 3
a
```

```bash
$ cargo run -r --bin run-js tests/example1.js
Got Value: 4
```
