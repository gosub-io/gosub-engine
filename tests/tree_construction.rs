use gosub_engine::testing::tree_construction::fixture_from_filename;
use lazy_static::lazy_static;
use std::collections::HashSet;
use test_case::test_case;

const DISABLED_CASES: &[&str] = &[
    // tests2.dat
    "<!DOCTYPE html><frameset> te st",
    "<!DOCTYPE html><frameset></frameset> te st",
    "<!DOCTYPE html><table><tr>TEST",
    "<!doctypehtml><scrIPt type=text/x-foobar;baz>X</SCRipt",
    "testtest",
    r#"<!DOCTYPE html><body><title>X</title><meta name=z><link rel=foo><style>x { content:"</style" } </style>"#,
    "<!DOCTYPE html><script></script>  <title>x</title>  </head>",
    "<!DOCTYPE html><html><body><html id=x>",
    r#"<!DOCTYPE html>X</body><html id="x">"#,
    "<!DOCTYPE html>X</html>",
    "<!DOCTYPE html>X<p/x/y/z>",
    "<!DOCTYPE <!DOCTYPE HTML>><!--<!--x-->-->",
    "<!doctype html><div><form></form><div></div></div>",
    // tests3.dat
    "<head></head><!-- -->x<style></style><!-- --><script></script>",
    "<!DOCTYPE html><html><head></head><body><pre>foo</pre></body></html>",
    "<!DOCTYPE html><html><head></head><body><pre>xy</pre></body></html>",
    "<!DOCTYPE html><html><head></head><body><pre>x<div>y</pre></body></html>",
    "<!DOCTYPE html><pre>&#x0a;&#x0a;A</pre>",
    "<!DOCTYPE html><textarea>foo</textarea>",
    // "<!DOCTYPE html><html><head></head><body><ul><li><div><p><li></ul></body></html>",
    "<p><table></table>",
    // tests5.dat
    "<style> <!-- </style> --> </style>x",
    "<script> <!-- </script> --> </script>x",
    "<title> <!-- </title> --> </title>x",
    "<textarea> <!--- </textarea>->x</textarea> --> </textarea>x",
    "<noscript><!--</noscript>--></noscript>",
    // tests6.dat
    "<body><div>",
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
#[test_case("tests5.dat")]
#[test_case("tests6.dat")]
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
