use gosub_engine::testing::tree_construction::fixture_from_filename;
use lazy_static::lazy_static;
use std::collections::HashSet;
use test_case::test_case;

const DISABLED_CASES: &[&str] = &[
    "<a X>0<b>1<a Y>2",
    "<!-----><font><div>hello<table>excite!<b>me!<th><i>please!</tr><!--X-->",
    "<!DOCTYPE html><li>hello<li>world<ul>how<li>do</ul>you</body><!--do-->",
    "<!DOCTYPE html><script> <!-- </script> --> </script> EOF",
    "<p id=a><b><p id=b></b>TEST",
    "<b id=a><p><b id=b></p></b>TEST",
    "<font><p>hello<b>cruel</font>world",
    "<DIV> abc <B> def <I> ghi <P> jkl </B>",
    "<DIV> abc <B> def <I> ghi <P> jkl </B> mno",
    "<DIV> abc <B> def <I> ghi <P> jkl </B> mno </I>",
    "<DIV> abc <B> def <I> ghi <P> jkl </B> mno </I> pqr",
    "<DIV> abc <B> def <I> ghi <P> jkl </B> mno </I> pqr </P>",
    "<DIV> abc <B> def <I> ghi <P> jkl </B> mno </I> pqr </P> stu",
    r#"<a href="blah">aba<table><a href="foo">br<tr><td></td></tr>x</table>aoe"#,
    r#"<a href="blah">aba<table><tr><td><a href="foo">br</td></tr>x</table>aoe"#,
    r#"<table><a href="blah">aba<tr><td><a href="foo">br</td></tr>x</table>aoe"#,
    "<a href=a>aa<marquee>aa<a href=b>bb</marquee>aa",
    "<table><tr><tr><td><td><span><th><span>X</table>",
    "<textarea><p></textarea>",
    "<ul><li></li><div><li></div><li><li><div><li><address><li><b><em></b><li></ul>",
    "<ul><li><ul></li><li>a</li></ul></li></ul>",
    "<p><b><div><marquee></p></b></div>X",
    "<a><table><td><a><table></table><a></tr><a></table><b>X</b>C<a>Y",
    "<wbr><strike><code></strike><code><strike></code>",
    "<p><b><div><marquee></p></b></div>",
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
