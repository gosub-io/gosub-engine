#![no_main]

use gosub_css3::Css3;
use gosub_interface::css3::CssOrigin;
use gosub_shared::config::ParserConfig;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let config = ParserConfig {
            ignore_errors: true,
            match_values: true,
            ..Default::default()
        };
        let _ = Css3::parse_str(s, config, CssOrigin::Author, "fuzz");
    }
});
