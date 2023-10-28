use gosub_engine::{
    testing::{
        self,
        tree_construction::{ErrorResult, NodeResult, SubtreeResult, Test, TestResult},
    },
    types::Result,
};

pub struct TestResults {
    /// Number of tests (as defined in the suite)
    tests: usize,
    /// Number of assertions (different combinations of input/output per test)
    assertions: usize,
    /// How many succeeded assertions
    succeeded: usize,
    /// How many failed assertions
    failed: usize,
    /// How many failed assertions where position is not correct
    failed_position: usize,
    /// The actual tests that have failed
    tests_failed: Vec<(usize, usize, String)>,
}

fn main() -> Result<()> {
    let mut results = TestResults {
        tests: 0,
        assertions: 0,
        succeeded: 0,
        failed: 0,
        failed_position: 0,
        tests_failed: Vec::new(),
    };

    let filenames = Some(&["tests19.dat"][..]);
    let fixtures = testing::tree_construction::fixtures(filenames).expect("fixtures");

    for fixture_file in fixtures {
        println!(
            "🏃‍♂️ Running {} tests from 🗄️ {:?}",
            fixture_file.tests.len(),
            fixture_file.path
        );

        let mut test_idx = 1;
        for test in fixture_file.tests {
            // if test_idx == 81 {
            run_tree_test(test_idx, &test, &mut results);
            // }
            test_idx += 1;
        }

        println!(
            "\
    🏁 Tests completed: Ran {} tests, {} assertions, {} succeeded, {} failed ({} position failures)\n",
            results.tests,
            results.assertions,
            results.succeeded,
            results.failed,
            results.failed_position
        );
    }

    if results.failed > 0 {
        println!("❌ Failed tests:");
        for (test_idx, line, data) in results.tests_failed {
            println!("  * Test #{} at line {}:", test_idx, line);
            println!("    {}", data);
        }
    }
    Ok(())
}

fn run_tree_test(test_idx: usize, test: &Test, all_results: &mut TestResults) {
    #[cfg(feature = "debug_parser_verbose")]
    println!(
        "🧪 Running test #{test_idx}: {}:{}",
        test.file_path, test.line
    );

    all_results.tests += 1;

    let results = test.run().expect("problem running tree construction test");

    for result in results {
        #[cfg(feature = "debug_parser")]
        print_test_result(&result);

        // Check the document tree, which counts as a single assertion
        all_results.assertions += 1;
        if result.success() {
            all_results.succeeded += 1;
        } else {
            all_results.failed += 1;
        }

        let errors = test.errors();

        if result.actual_errors.len() != errors.len() {
            #[cfg(feature = "debug_parser")]
            println!(
                "⚠️ Unexpected errors found (wanted {}, got {}): ",
                errors.len(),
                result.actual_errors.len()
            );

            // for want_err in &test.errors {
            //     println!(
            //         "     * Want: '{}' at {}:{}",
            //         want_err.code, want_err.line, want_err.col
            //     );
            // }
            // for got_err in &parse_errors {
            //     println!(
            //         "     * Got: '{}' at {}:{}",
            //         got_err.message, got_err.line, got_err.col
            //     );
            // }
            // results.assertions += 1;
            // results.failed += 1;
        } else {
            #[cfg(feature = "debug_parser")]
            println!("✅  Found {} errors", result.actual_errors.len());
        }

        // For now, we skip the tests that checks for errors as most of the errors do not match
        // with the actual tests, as these errors as specific from html5lib. Either we reuse them
        // or have some kind of mapping to our own errors if we decide to use our custom errors.

        // // Check each error messages
        // let mut idx = 0;
        // for error in &test.errors {
        //     if parse_errors.get(idx).is_none() {
        //         println!("❌ Expected error '{}' at {}:{}", error.code, error.line, error.col);
        //         results.assertions += 1;
        //         results.failed += 1;
        //         continue;
        //     }
        //
        //     let err = parse_errors.get(idx).unwrap();
        //     let got_error = Error{
        //         code: err.message.to_string(),
        //         line: err.line as i64,
        //         col: err.col as i64,
        //     };
        //
        //     match match_error(&got_error, &error) {
        //         ErrorResult::Failure => {
        //             results.assertions += 1;
        //             results.failed += 1;
        //         },
        //         ErrorResult::PositionFailure => {
        //             results.assertions += 1;
        //             results.failed += 1;
        //             results.failed_position += 1;
        //         },
        //         ErrorResult::Success => {
        //             results.assertions += 1;
        //             results.succeeded += 1;
        //         }
        //     }
        //
        //     idx += 1;
        // }

        // Display additional data if there a failure is found
        if !result.success() {
            all_results
                .tests_failed
                .push((test_idx, test.line, test.data().to_string()));

            if cfg!(feature = "debug_parser") {
                println!("----------------------------------------");
                println!("📄 Input stream: ");
                println!("{}", test.data());
                println!("----------------------------------------");
                println!("🌳 Generated tree: ");
                println!("{}", result.actual_document);
                println!("----------------------------------------");
                println!("🌳 Expected tree: ");
                for line in &test.document {
                    println!("{line}");
                }
            }
        }

        #[cfg(feature = "debug_parser")]
        println!("----------------------------------------");
    }
}

#[allow(dead_code)]
fn print_test_result(result: &TestResult) {
    // We need a better tree match system. Right now we match the tree based on the (debug) output
    // of the tree. Instead, we should generate a document-tree from the expected output and compare
    // it against the current generated tree.
    print_node_result(&result.root)
}

#[allow(dead_code)]
fn print_node_result(result: &SubtreeResult) {
    match &result.node {
        Some(NodeResult::ElementMatchSuccess { actual }) => {
            println!("✅  {actual}");
        }

        Some(NodeResult::AttributeMatchFailure { name, expected, .. }) => {
            println!("❌ {expected}, Found unexpected attribute: {name}");
        }

        Some(NodeResult::ElementMatchFailure { name, expected, .. }) => {
            println!("❌ {expected}, Found unexpected element node: {name}");
        }

        Some(NodeResult::TextMatchSuccess { expected }) => {
            println!("✅  {expected}");
        }

        Some(NodeResult::TextMatchFailure { expected, text, .. }) => {
            println!("❌ {expected}, Found unexpected text node: {text}");
        }

        Some(NodeResult::CommentMatchFailure {
            expected, comment, ..
        }) => {
            println!("❌ {expected}, Found unexpected comment node: {comment}");
        }

        None => {}
    }

    result.children.iter().for_each(print_node_result);
}

#[allow(dead_code)]
fn match_error(result: ErrorResult) {
    match result {
        ErrorResult::Success { actual } => {
            println!(
                "✅  Found parse error '{}' at {}:{}",
                actual.code, actual.line, actual.col
            );
        }

        ErrorResult::Failure { expected, .. } => {
            println!(
                "❌ Expected error '{}' at {}:{}",
                expected.code, expected.line, expected.col
            );
        }

        ErrorResult::PositionFailure { actual, expected } => {
            // Found an error with the same code, but different line/pos
            println!(
                "⚠️ Unexpected error position '{}' at {}:{} (got: {}:{})",
                expected.code, expected.line, expected.col, actual.line, actual.col
            );
        }
    }
}
