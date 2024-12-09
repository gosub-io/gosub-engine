# Gosub crates

The engine is split up in a few different crates. This is done to keep the codebase clean and to allow for easier testing and development. The following crates are currently available:

* gosub_config
* gosub_css3
* gosub_html5
* gosub_jsapi
* gosub_net
* gosub_render_utils
* gosub_renderer
* gosub_shared
* gosub_svg
* gosub_taffy
* gosub_testing
* gosub_typeface
* gosub_useragent
* gosub_v8
* gosub_vello
* gosub_webexecutor
* gosub_webinterop

Some of the crates are dependent on other crates, but we aim to be as modular as possible. The `gosub_shared` crate is a crate that is used by most of the other crates and contains shared code and data structures.


## gosub_config
This crate contains a configuration system that is used by the engine. It can store information in a store (for instance, sqlite, or simply json) and can 

## gosub_css3
This crate contains a CSS3 parser that can parse CSS3 stylesheets and can be used to style HTML5 documents. It also holds the parser to parse the CSS3 property syntax in order to validate Css properties. 

## gosub_html5
The main html5 tokenizer and parser. It also includes the main "Document" object that is used to represent the DOM tree and its node elements

## gosub_jsapi
This crate contains Javascript api's that are usable in the browser. For instance, the console API, the fetch API, the DOM API, etc. 

## gosub_net
This crate contains the network stack that is used to fetch resources from the web. It can fetch resources from the web, but also from the local filesystem. Currently hosting a DNS system that we can use for resolving domain names over different kind of protocols.

## gosub_render_utils
This crate contains implementations of the render tree and some other utilities, for instance for resolving mouse positions back to elements.

## gosub_renderer
This crate contains the actual renderer.

## gosub_shared
Some of the code and data structures that will be used throughout different crates are stored here. It also holds the traits that are used to implement the different parts of the engine.

## gosub_svg
Implementation of the SVG Document for `usvg` and optionally the `resvg` crates, used for SVG rendering.

## gosub_taffy
Implementation of layout traits for the `taffy` layouting system.

## gosub_testing
A dedicated crate for testing some of the engine. This will allow to easily test the different parts of the engine, most notably the html5 tokenizer and parser.

## gosub_typeface
Currently doesn't do much, but it is used to store fallback fonts and the `Font` trait

## gosub_useragent
This crate keeps a simple application with event loop renders html5 documents. It can be seen as a very simple browser. Ultimately, this crate will be removed in favor of an external application that will use the engine. 

## gosub_v8
Gosub bindings to the V8 javascript engine.

## gosub_vello
Implementation of a RenderBackend for the `vello` crate

## gosub_webexecutor
System to execute javascript. This could also be used for executing other languages in the future, like lua.

## gosub_webinterop
Proc macro to easily pass functions and define APIs to javascript, wasm or lua and others.