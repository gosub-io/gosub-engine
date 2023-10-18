use gosub_engine::testing::tree_construction::fixture_from_filename;
use lazy_static::lazy_static;
use std::collections::HashSet;
use test_case::test_case;

const DISABLED_CASES: &[&str] = &[
    "<!-----><font><div>hello<table>excite!<b>me!<th><i>please!</tr><!--X-->",
    "<!DOCTYPE html><li>hello<li>world<ul>how<li>do</ul>you</body><!--do-->",
    "<!DOCTYPE html><script> <!-- </script> --> </script> EOF",
    r#"<a href="blah">aba<table><a href="foo">br<tr><td></td></tr>x</table>aoe"#,
    r#"<a href="blah">aba<table><tr><td><a href="foo">br</td></tr>x</table>aoe"#,
    r#"<table><a href="blah">aba<tr><td><a href="foo">br</td></tr>x</table>aoe"#,
    "<table><tr><tr><td><td><span><th><span>X</table>",
    "<ul><li></li><div><li></div><li><li><div><li><address><li><b><em></b><li></ul>",
    "<ul><li><ul></li><li>a</li></ul></li></ul>",
];

lazy_static! {
    static ref DISABLED: HashSet<String> = DISABLED_CASES
        .iter()
        .map(|s| s.to_string())
        .collect::<HashSet<_>>();
}

// See tests/data/html5lib-tests/tree-construction/ for other test files.
#[test_case("tests1.dat")]
fn tree_construction(filename: &str) {
    let fixture_file = fixture_from_filename(filename).expect("fixture");

    for test in fixture_file.tests {
        if DISABLED.contains(&test.data) {
            // Check that we don't panic
            let _ = test.parse().expect("problem parsing");
            continue;
        }

        println!("tree construction: {}", test.data);
        test.assert_valid();
    }
}
