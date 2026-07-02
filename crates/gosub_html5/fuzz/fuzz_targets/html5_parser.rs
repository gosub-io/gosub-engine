#![no_main]

use gosub_css3::system::Css3System;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::html_compile;
use gosub_html5::parser::Html5Parser;
use gosub_interface::config::ModuleConfiguration;
use libfuzzer_sys::fuzz_target;

#[derive(Clone, Debug, PartialEq)]
struct Config;

impl ModuleConfiguration for Config {
    type CssSystem = Css3System;
    type Document = DocumentImpl<Self>;
    type HtmlParser = Html5Parser<'static, Self>;
}

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = html_compile::<Config>(s);
    }
});
