use gosub_engine::testing::tree_construction::fixture_from_filename;
use lazy_static::lazy_static;
use std::collections::HashSet;
use test_case::test_case;

const DISABLED_CASES: &[&str] = &[
    // tests18.dat
    "<!doctype html><template><plaintext>a</template>b",
];

lazy_static! {
    static ref DISABLED: HashSet<String> = DISABLED_CASES
        .iter()
        .map(|s| s.to_string())
        .collect::<HashSet<_>>();
}

// See tests/data/html5lib-tests/tree-construction/ for other test files.
#[test_case("tests1.dat")]
#[test_case("tests2.dat")]
#[test_case("tests3.dat")]
#[test_case("tests4.dat")]
#[test_case("tests5.dat")]
#[test_case("tests6.dat")]
#[test_case("tests7.dat")]
#[test_case("tests8.dat")]
#[test_case("tests9.dat")]
#[test_case("tests10.dat")]
#[test_case("tests11.dat")]
#[test_case("tests12.dat")]
#[test_case("tests14.dat")]
#[test_case("tests15.dat")]
#[test_case("tests16.dat")]
#[test_case("tests17.dat")]
#[test_case("tests18.dat")]
#[test_case("tests19.dat")]
#[test_case("tests20.dat")]
// #[test_case("tests21.dat")]
#[test_case("tests22.dat")]
#[test_case("tests23.dat")]
#[test_case("tests24.dat")]
#[test_case("tests25.dat")]
#[test_case("tests26.dat")]
#[test_case("adoption01.dat")]
#[test_case("adoption02.dat")]
#[test_case("blocks.dat")]
#[test_case("comments01.dat")]
#[test_case("doctype01.dat")]
// #[test_case("domjs-unsafe.dat")]
#[test_case("entities01.dat")]
#[test_case("entities02.dat")]
// #[test_case("foreign-fragment.dat")]
#[test_case("html5test-com.dat")]
#[test_case("inbody01.dat")]
#[test_case("isindex.dat")]
#[test_case("main-element.dat")]
#[test_case("math.dat")]
#[test_case("menuitem-element.dat")]
#[test_case("namespace-sensitivity.dat")]
#[test_case("noscript01.dat")]
#[test_case("pending-spec-changes.dat")]
#[test_case("pending-spec-changes-plain-text-unsafe.dat")]
// #[test_case("plain-text-unsafe.dat")]
#[test_case("quirks01.dat")]
#[test_case("ruby.dat")]
#[test_case("scriptdata01.dat")]
#[test_case("search-element.dat")]
#[test_case("svg.dat")]
#[test_case("tables01.dat")]
// #[test_case("template.dat")]
#[test_case("tests_innerHTML_1.dat")]
#[test_case("tricky01.dat")]
#[test_case("webkit01.dat")]
#[test_case("webkit02.dat")]
fn tree_construction(filename: &str) {
    let fixture_file = fixture_from_filename(filename).expect("fixture");

    for test in fixture_file.tests {
        if DISABLED.contains(test.data()) {
            for &scripting_enabled in test.script_modes() {
                // Check that we don't panic
                let _ = test.parse(scripting_enabled).expect("problem parsing");
            }
            continue;
        }

        println!("tree construction: {}", test.data());
        test.assert_valid();
    }
}
