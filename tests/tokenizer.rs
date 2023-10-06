use lazy_static::lazy_static;
use std::collections::HashSet;
use test_case::test_case;

use gosub_engine::testing::tokenizer;

const DISABLED_CASES: &[&str] = &[
    // TODO: Handle UTF-16 high and low private surrogate characters
    // https://www.compart.com/en/unicode/U+DBC0
    // https://www.compart.com/en/unicode/U+DC00
    ";\\uDBC0\\uDC00",
    "<!-- -\\uDBC0\\uDC00",
    "<!-- \\uDBC0\\uDC00",
    "<!----\\uDBC0\\uDC00",
    "<!---\\uDBC0\\uDC00",
    "<!--\\uDBC0\\uDC00",
    "<!DOCTYPE a PUBLIC\"\\uDBC0\\uDC00",
    "<!DOCTYPE a PUBLIC'\\uDBC0\\uDC00",
    "<!DOCTYPE a SYSTEM\"\\uDBC0\\uDC00",
    "<!DOCTYPE a SYSTEM'\\uDBC0\\uDC00",
    "<!DOCTYPE a\\uDBC0\\uDC00",
    "<!DOCTYPE \\uDBC0\\uDC00",
    "<!DOCTYPEa PUBLIC\"\\uDBC0\\uDC00",
    "<!DOCTYPEa PUBLIC'\\uDBC0\\uDC00",
    "<!DOCTYPEa SYSTEM\"\\uDBC0\\uDC00",
    "<!DOCTYPEa SYSTEM'\\uDBC0\\uDC00",
    "<!DOCTYPEa\\uDBC0\\uDC00",
    "<!DOCTYPE\\uDBC0\\uDC00",
    "\\uDBC0\\uDC00",
];

lazy_static! {
    static ref DISABLED: HashSet<String> = DISABLED_CASES
        .iter()
        .map(|s| s.to_string())
        .collect::<HashSet<_>>();
}

#[test_case("contentModelFlags.test")]
#[test_case("domjs.test")]
#[test_case("entities.test")]
#[test_case("escapeFlag.test")]
#[test_case("namedEntities.test")]
#[test_case("numericEntities.test")]
#[test_case("pendingSpecChanges.test")]
#[test_case("test1.test")]
#[test_case("test2.test")]
#[test_case("test3.test")]
#[test_case("test4.test")]
// #[test_case("unicodeCharsProblematic.test")]
#[test_case("unicodeChars.test")]
// #[test_case("xmlViolation.test")]
fn tokenization(filename: &str) {
    let root = tokenizer::fixture_from_filename(filename).unwrap();

    for test in root.tests {
        if DISABLED.contains(&test.description) {
            // Check that we don't panic
            test.tokenize();
            continue;
        }

        test.assert_valid();
    }
}
