# GoSub: Gateway to Optimized Searching and Unlimited Browsing

This repository holds the GoSub HTML5 parser/tokenizer. It is a standalone library that can be used by other projects but will ultimately be used by the GoSub browser. See the [About](#about) section for more information.

```
                       _     
                      | |    
  __ _  ___  ___ _   _| |__  
 / _` |/ _ \/ __| | | | '_ \ 
| (_| | (_) \__ \ |_| | |_) |
 \__, |\___/|___/\__,_|_.__/ 
  __/ |  The Gateway to                    
 |___/   Optimized Searching and 
         Unlimited Browsing                    
```

## About

This repository is part of the GoSub browser project. Currently, there is only a single component/repository (this one), but the idea will be that there are many other components that, as a whole, make up a full-fledged browser. Each of the components can probably function as something standalone (ie, html5 parser, CSS parser, etc.).

In the future, this component (HTML5 parser) will receive a stream of bytes through an API and output a stream of events. The next component will consume the events, and so on, until we can display something in a window/user agent. This could be a text-mode browser, but the idea is to have a graphical browser.

## Status

> This project is in its infancy. There is no browser you can use yet.

This is a work in progress. The current status is that the parser can parse a few HTML5 documents, but it is far from ready. The main goal is to be able to parse correctly all the tests in the html5lib-tests repository (https://github.com/html5lib/html5lib-tests).

Our goal at the moment is to research as much as possible and to setup proof-of-concepts in order to gain more understanding in the field of browsers. We are not trying to create a full-fledged browser at the moment, but it will be our ultimate goal.

## How to build

This project uses [cargo](https://doc.rust-lang.org/cargo/) and [rustup](https://www.rust-lang.org/tools/install). First you must install `rustup` at the link provided. After installing `rustup`, run:

```bash
$ rustup toolchain install 1.73
$ rustc --version
rustc 1.73.0 (cc66ad468 2023-10-03)
```

Once Rust is installed, run this command to build the project:

```bash
$ cargo build
```

Doing this will create the following binaries:

| File                              | Type | Description                                                     |
|-----------------------------------|------|-----------------------------------------------------------------|
| `target/debug/gosub-parser`       | bin  | The actual html5 parser/tokenizer                               |
| `target/debug/parser-test`        | bin  | A test suite for the parser that tests specific tests           |
| `target/debug/html5-parser-tests` | bin  | A test suite that tests all html5lib tests for the treebuilding |
| `target/debug/test-user-agent`    | bin  | A simple placeholder user agent for testing purposes            |

You can then run the binaries like so:

```bash
$ ./target/debug/gosub-parser https://news.ycombinator.com/
$ ./target/debug/parser-test
```

To build the release build, run:

```bash
$ cargo build --release
$ ./target/release/gosub-parser https://news.ycombinator.com/
```

To run the tests and benchmark suite, do:

```bash
$ make test
$ cargo bench
$ ls target/criterion/report 
index.html
```