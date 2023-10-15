use gosub_engine::testing::tree_construction::fixture_from_filename;
use lazy_static::lazy_static;
use std::collections::HashSet;
use test_case::test_case;

const DISABLED_CASES: &[&str] = &[];

lazy_static! {
    static ref DISABLED: HashSet<String> = DISABLED_CASES
        .iter()
        .map(|s| s.to_string())
        .collect::<HashSet<_>>();
}

#[test_case("tests1.dat")]
fn tree_construction(filename: &str) {
    let fixture_file = fixture_from_filename(filename).expect("fixture");

    for test in fixture_file.tests {
        if DISABLED.contains(&test.data) {
            // Check that we don't panic
            test.run();
            continue;
        }

        test.assert_valid();
    }
}
