use gosub_engine::testing::tree_construction::fixture_from_filename;
use lazy_static::lazy_static;
use std::collections::HashSet;
use test_case::test_case;

const DISABLED_CASES: &[&str] = &[
    // tests2.dat
    "<!DOCTYPE html>X<p/x/y/z>",
    // tests4.dat
    "</plaintext>",
    "direct div content",
    "direct textarea content",
    "textarea content with <em>pseudo</em> <foo>markup",
    "this is &#x0043;DATA inside a <style> element",
    "setting html's innerHTML",
    "<title>setting head's innerHTML</title>",
    "direct <title> content",
    "<!-- inside </script> -->",
    // tests6.dat
    "<body>\n<div>",
    "<frameset></frameset><noframes>",
    "</caption><div>",
    "</table><div>",
    "</table></tbody></tfoot></thead></tr><div>",
    "<table><colgroup>foo",
    "foo<col>",
    "</frameset><frame>",
    "</body><div>",
    "</tr><td>",
    "</tbody></tfoot></thead><td>",
    "<caption><col><colgroup><tbody><tfoot><thead><tr>",
    "</table><tr>",
    "<body></body></html>",
    r#"<!DOCTYPE html PUBLIC "-//W3C//DTD HTML 4.01//EN"><html></html>"#,
    "<param><frameset></frameset>",
    "<source><frameset></frameset>",
    "<track><frameset></frameset>",
    "</html><frameset></frameset>",
    "</body><frameset></frameset>",
<<<<<<< HEAD
    // tests7.dat
    "<body>X</body></body>",
=======
    // tests8.dat
    "x<table>x",
    "x<table><table>x",
    // tests10.dat
    "<!DOCTYPE html><body xlink:href=foo xml:lang=en><svg><g xml:lang=en xlink:href=foo />bar</svg>",
    "<div><svg><path><foreignObject><math></div>a",
    "<div><svg><path><foreignObject><p></div>a",
    "<!DOCTYPE html><p><svg><desc><p>",
    "<!DOCTYPE html><p><svg><title><p>",
    "<svg><script></script><path>",
    // tests24.dat
    "<!DOCTYPE html>&ThickSpace;A",
>>>>>>> a75314c (Fixed foreign elements (svg, mathml))
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
<<<<<<< HEAD
#[test_case("tests15.dat")]
=======
#[test_case("tests10.dat")]
>>>>>>> a75314c (Fixed foreign elements (svg, mathml))
#[test_case("tests16.dat")]
#[test_case("tests17.dat")]
#[test_case("tests22.dat")]
#[test_case("tests24.dat")]
#[test_case("tests25.dat")]
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
