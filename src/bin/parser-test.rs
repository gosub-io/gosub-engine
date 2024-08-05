use gosub_testing::testing::tree_construction::fixture::read_fixtures;
use gosub_testing::testing::tree_construction::result::ResultStatus;
use gosub_testing::testing::tree_construction::Harness;
use gosub_testing::testing::tree_construction::Test;

/// Holds the results from all tests that are executed
#[derive(Default)]
pub struct TotalTestResults {
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

fn main() {
    let mut results = TotalTestResults::default();

    let filenames = Some(&["tests15.dat"][..]);
    let fixtures = read_fixtures(filenames).expect("fixtures");

    for fixture_file in fixtures {
        println!(
            "🏃‍♂️ Running {} tests from 🗄️ {:?}",
            fixture_file.tests.len(),
            fixture_file.path
        );

        let mut test_idx = 1;
        for test in fixture_file.tests {
            if test_idx == 10 {
                run_test(test_idx, test, &mut results);
            }
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
            println!("  * Test #{test_idx} at line {line}:");
            println!("    {data}");
        }
    }
}

fn run_test(test_idx: usize, test: Test, all_results: &mut TotalTestResults) {
    #[cfg(all(feature = "debug_parser_verbose", test))]
    println!(
        "🧪 Running test #{test_idx}: {}:{}",
        test.file_path, test.line
    );

    all_results.tests += 1;

    let mut harness = Harness::new();
    let result = harness
        .run_test(test.clone(), false)
        .expect("problem parsing");

    // #[cfg(all(feature = "debug_parser", not(test)))]
    // print_test_result(&result);

    for entry in &result.tree_results {
        all_results.assertions += 1;

        match entry.result {
            ResultStatus::Success => {
                all_results.succeeded += 1;

                #[cfg(all(feature = "debug_parser", test))]
                println!("✅  {}", entry.actual);
            }
            ResultStatus::Missing => {
                all_results.failed += 1;

                #[cfg(all(feature = "debug_parser", test))]
                println!("❌ {} (missing)", entry.expected);
            }
            ResultStatus::Additional => {
                all_results.failed += 1;

                #[cfg(all(feature = "debug_parser", test))]
                println!("❌ {} (unexpected)", entry.actual);
            }
            ResultStatus::Mismatch => {
                all_results.failed += 1;

                #[cfg(all(feature = "debug_parser", test))]
                println!("❌ {} (wanted: {})", entry.actual, entry.expected);
            }
            ResultStatus::IncorrectPosition => {}
        }
    }

    for entry in &result.error_results {
        all_results.assertions += 1;

        match entry.result {
            ResultStatus::Success => {
                all_results.succeeded += 1;

                #[cfg(all(feature = "debug_parser", test))]
                println!(
                    "✅  ({}:{}) {}",
                    entry.actual.line, entry.actual.col, entry.actual.message
                );
            }
            ResultStatus::Missing => {
                all_results.failed += 1;

                #[cfg(all(feature = "debug_parser", test))]
                println!(
                    "❌ ({}:{}) {} (missing)",
                    entry.expected.line, entry.expected.col, entry.expected.message
                );
            }
            ResultStatus::Additional => {
                all_results.failed += 1;

                #[cfg(all(feature = "debug_parser", test))]
                println!(
                    "❌ ({}:{}) {} (unexpected)",
                    entry.actual.line, entry.actual.col, entry.actual.message
                );
            }
            ResultStatus::Mismatch => {
                all_results.failed += 1;

                #[cfg(all(feature = "debug_parser", test))]
                println!(
                    "❌ ({}:{}) {} (wanted: {})",
                    entry.actual.line,
                    entry.actual.col,
                    entry.actual.message,
                    entry.expected.message
                );
            }
            ResultStatus::IncorrectPosition => {
                all_results.failed += 1;
                all_results.failed_position += 1;

                #[cfg(all(feature = "debug_parser", test))]
                println!(
                    "❌ ({}:{}) (wanted: ({}::{})) {}",
                    entry.actual.line,
                    entry.actual.col,
                    entry.expected.line,
                    entry.expected.col,
                    entry.expected.message
                );
            }
        }
    }

    // // Display additional data if there a failure is found
    if !result.is_success() {
        all_results
            .tests_failed
            .push((test_idx, test.line, test.document_as_str().to_string()));
    }
}
