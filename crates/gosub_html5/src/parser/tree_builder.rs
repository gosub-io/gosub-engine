use crate::parser::NodeId;
use gosub_shared::byte_stream::Location;
use gosub_shared::types::Result;

/// TreeBuilder is an interface to abstract DOM tree modifications.
///
/// This is implemented by DocumentHandle to support direct immediate manipulation of the DOM
/// and implemented by DocumentTaskQueue to support queueing up several mutations to be performed at once.
pub trait TreeBuilder {
    /// Create a new element node with the given tag name and append it to a parent
    /// with an optional position parameter which places the element at a specific child index.
    fn create_element(
        &mut self,
        name: &str,
        parent_id: NodeId,
        position: Option<usize>,
        namespace: &str,
        location: Location,
    ) -> NodeId;

    /// Create a new text node with the given content and append it to a parent.
    fn create_text(&mut self, content: &str, parent_id: NodeId, location: Location) -> NodeId;

    /// Create a new comment node with the given content and append it to a parent.
    fn create_comment(&mut self, content: &str, parent_id: NodeId, location: Location) -> NodeId;

    /// Insert/update an attribute for an element node.
    fn insert_attribute(
        &mut self,
        key: &str,
        value: &str,
        element_id: NodeId,
        location: Location,
    ) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use gosub_testing::testing::tree_construction::fixture::{
        fixture_root_path, read_fixture_from_path,
    };
    use gosub_testing::testing::tree_construction::Harness;
    use test_case::test_case;

    const DISABLED_CASES: &[&str] = &[
        // tests18.dat
        "<!doctype html><template><plaintext>a</template>b",
    ];

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
    #[test_case("tests21.dat")]
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
    #[test_case("domjs-unsafe.dat")]
    #[test_case("entities01.dat")]
    #[test_case("entities02.dat")]
    #[test_case("foreign-fragment.dat")]
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
    #[test_case("plain-text-unsafe.dat")]
    #[test_case("quirks01.dat")]
    #[test_case("ruby.dat")]
    #[test_case("scriptdata01.dat")]
    #[test_case("search-element.dat")]
    #[test_case("svg.dat")]
    #[test_case("tables01.dat")]
    #[test_case("template.dat")]
    #[test_case("tests_innerHTML_1.dat")]
    #[test_case("tricky01.dat")]
    #[test_case("webkit01.dat")]
    #[test_case("webkit02.dat")]
    fn tree_construction(filename: &str) {
        let fixture_file =
            read_fixture_from_path(fixture_root_path().join(filename)).expect("fixture");
        let mut harness = Harness::new();

        for test in fixture_file.tests {
            // skip disabled tests
            if DISABLED_CASES.contains(&test.document_as_str()) {
                continue;
            }

            // for each test, run it with and without scripting enabled based on the test file
            for &scripting_enabled in test.script_modes() {
                let result = harness
                    .run_test(test.clone(), scripting_enabled)
                    .expect("problem parsing");

                println!(
                    "tree construction: {}:{}\n{}",
                    test.file_path,
                    test.line,
                    test.document_as_str()
                );
                assert!(result.is_success());
            }
        }
    }
}
