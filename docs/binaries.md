# Gosub binaries

The current engine is supported with a few binaries to test out some of the different components. They are by themselves 
stand-alone binaries but they have not a lot of use besides testing and experimenting with the engine.


## config-store

The `config-store` allows you to view and modify the current config-store system found in the engine.

```bash

$ cargo run -r --bin config-store list

dns.cache.max_entries                   : u:1000
dns.cache.ttl.override.enabled          : b:false
dns.cache.ttl.override.seconds          : u:0
dns.local.enabled                       : b:true
dns.local.table                         : m: ''
dns.remote.doh.enabled                  : b:false
dns.remote.dot.enabled                  : b:false
dns.remote.nameservers                  : m: ''
dns.remote.retries                      : u:3
dns.remote.timeout                      : u:5
dns.remote.use_hosts_file               : b:true
useragent.default_page                  : s:about:blank
useragent.tab.close_button              : m: left
useragent.tab.max_opened                : i:-1
renderer.opengl.enabled                 : b:true


$ cargo run -r --bin config-store search --key 'user*'
useragent.default_page                  : s:about:blank
useragent.tab.close_button              : m: left
useragent.tab.max_opened                : i:-1

```


## css3-parser

The `css3-parser` will try and parse a CSS stylesheet and displays any errors it find or shows the parsed css tree.

```css 
div, a {
  color: white;
  border: 1px solid black;
}
```

```bash
$ cargo run -r --bin css3-parser file://tests/data/css3-data/test.css

[Stylesheet (1)]
  [Rule]
    [SelectorList (2)]
      [Selector]
        [Ident] div
      [Selector]
        [Combinator]
        [Ident] a
    [Block]
      [Declaration] property: color important: false
        [Ident] white
      [Declaration] property: border important: false
        [Dimension] 1px
        [Ident] solid
        [Ident] black 
```

It does not test properties to see if their syntax match. So it will parse `color: 1%` as a valid line.


## gosub-parser

Fetches a URL, parses it and returns information about the process. It will return any information about stylesheets loaded, timings and displays the 
body of the fetched page

```bash

$ cargo run -r --bin gosub-parser https://news.ycombinator.com

Parsing url: Url { scheme: "https", cannot_be_a_base: false, username: "", password: None, host: Some(Domain("news.ycombinator.com")), port: None, path: "/", query: None, fragment: None }

Found 1 stylesheets
Stylesheet location: "https://news.ycombinator.com/news.css?evaBHzX7ZyR20JbMfele"

Parse Error: expected-doctype-but-got-start-tag
Parse Error: link element with rel attribute 'icon' is not supported in the body
Parse Error: link element with rel attribute 'alternate' is not supported in the body
Parse Error: anything else not allowed in after body insertion mode

Namespace            |    Count |      Total |        Min |        Max |        Avg |        50% |        75% |        95% |        99%
----------------------------------------------------------------------------------------------------------------------------------------
html5.parse          |        1 |      605ms |      605ms |      605ms |      605ms |      605ms |      605ms |      605ms |      605ms
                     |        1 |      605ms | https://news.ycombinator.com/
css3.parse           |        1 |      613µs |      613µs |      613µs |      613µs |      613µs |      613µs |      613µs |      613µs
                     |        1 |      613µs | https://news.ycombinator.com/news.css?evaBHzX7ZyR20JbMfele
...

```


```bash

$ cargo run -r --bin gosub-parser https://gosub.io

Parsing url: Url { scheme: "https", cannot_be_a_base: false, username: "", password: None, host: Some(Domain("gosub.io")), port: None, path: "/", query: None, fragment: None }

Found 2 stylesheets
Stylesheet location: "https://gosub.io/#inline"
Stylesheet location: "https://cdnjs.cloudflare.com/ajax/libs/font-awesome/6.4.2/css/all.min.css"

Parse Error: link element with rel attribute 'apple-touch-icon' is not supported in the body
Parse Error: link element with rel attribute 'icon' is not supported in the body
Parse Error: link element with rel attribute 'icon' is not supported in the body
Parse Error: link element with rel attribute 'manifest' is not supported in the body

Namespace            |    Count |      Total |        Min |        Max |        Avg |        50% |        75% |        95% |        99%
----------------------------------------------------------------------------------------------------------------------------------------
html5.parse          |        1 |      117ms |      117ms |      117ms |      117ms |      117ms |      117ms |      117ms |      117ms
                     |        1 |      117ms | https://gosub.io/
css3.parse           |        2 |        7ms |      101µs |        7ms |        3ms |        7ms |        7ms |        7ms |        7ms
                     |        1 |      101µs | https://gosub.io/#inline
                     |        1 |        7ms | https://cdnjs.cloudflare.com/ajax/libs/font-awesome/6.4.2/css/all.min.css
                     
```


## html5-parser-test

Runs the html5 test suite from the commandline. Might actually not function because it might not be able to find the testsuite files. See [this issue](https://github.com/gosub-io/gosub-engine/issues/521)


## parser-test

Runs the html5 parser test suite from the commandline. Might actually not function because it might not be able to find the testsuite files. See [this issue](https://github.com/gosub-io/gosub-engine/issues/521)


## renderer

A simple (graphical) renderer that tries to render the given url.


## run-js

Runs (simple) javascripts through the v8 engine. There is no connection with api's so `console.log` wont work.

```javascript
var a = 1 + 3

a
```

```bash

$ cargo run -r --bin run-js tests/example1.js
Got Value: 4
```


## display-text-tree

Generates a textual representation of a given website. Basically it will print all the text nodes from the page.

```bash

$ cargo run -r --bin display-text-tree https://gosub.io
```