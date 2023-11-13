# GoSub: Gateway to Optimized Searching and Unlimited Browsing

This repository holds the GoSub browser engine. It will become a standalone library that can be used by other projects but will ultimately be used by the Gosub browser user-agent. See the [About](#about) section for more information.

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

This repository is part of the Gosub browser project. This is the main engine that holds at least the following components:

 - HTML5 tokenizer / parser
 - CSS3 tokenizer / parser
 - Document tree
 - Several APIs for connecting to javascript
 - Configuration store

The idea is that this engine will receive some kind of stream of bytes (most likely from a socket or file) and parse this into a valid HTML5 document tree. From that point, it can be fed to a renderer engine that will render the document tree into a window, or it can be fed to a more simplistic engine that will render it in a terminal.

## Status

> This project is in its infancy. There is no browser you can use yet, but parsing a file into a document tree is possible.

The main goal for the parser is to be able to parse correctly all the tests in the html5lib-tests repository (https://github.com/html5lib/html5lib-tests). This is currently achieved for both the tokenizer tests and the tree-construction tests. There are a few small issues left which are mainly because of handling of UTF-16 characters and test that does dom modification through scripts (there is no javascript engine implemented yet).

From a parsing point of view, the html5 parser and most of the css3 parser is completed for now.

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

| File                              | Type | Description                                                                                                                                                     |
|-----------------------------------|------|-----------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `target/debug/gosub-parser`       | bin  | The actual html5 parser/tokenizer that allows you to convert html5 into a document tree.                                                                        
| `target/debug/parser-test`        | bin  | A test suite for the parser that tests specific tests. This will be removed as soon as the parser is completely finished as this tool is for developement only. 
| `target/debug/html5-parser-tests` | bin  | A test suite that tests all html5lib tests for the treebuilding                                                                                                 |
| `target/debug/test-user-agent`    | bin  | A simple placeholder user agent for testing purposes                                                                                                            |
| `target/debug/config-store`       | bin  | A simple test application of the config store for testing purposes                                                                                              |

You can then run the binaries like so:

```bash
$ ./target/debug/gosub-parser https://news.ycombinator.com/
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


## Contributing to the project
We welcome contributions to this project but the current status makes that we are spending a lot of time researching, building small proof-of-concepts and figuring out what needs to be done next. Much time of a contributor at this stage of the project will be non-coding.

We do like to hear from you if you are interested in contributing to the project and you can join us currently at our slack channel: [https://join.slack.com/t/gosubgroup/shared_invite/zt-248ksp6sy-9GUHEuf7BIlI6gxjcXJFnA 
](https://join.slack.com/t/gosubgroup/shared_invite/zt-2766v78v4-8NAtD1UU~jQzEQfzfP1sFg)https://join.slack.com/t/gosubgroup/shared_invite/zt-2766v78v4-8NAtD1UU~jQzEQfzfP1sFg
