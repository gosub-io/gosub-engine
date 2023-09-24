use std::{env, fs, io};
use std::fs::File;
use regex::Regex;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use gosub_engine::html5_parser::input_stream::InputStream;
use gosub_engine::html5_parser::node::NodeData;
use gosub_engine::html5_parser::parser::Html5Parser;
use gosub_engine::html5_parser::parser::document::Document;

pub struct TestResults{
    tests: usize,               // Number of tests (as defined in the suite)
    assertions: usize,          // Number of assertions (different combinations of input/output per test)
    succeeded: usize,           // How many succeeded assertions
    failed: usize,              // How many failed assertions
    failed_position: usize,     // How many failed assertions where position is not correct
}

struct Test {
    file_path: String,                  // Filename of the test
    line: usize,                        // Line number of the test
    data: String,                       // input stream
    errors: Vec<Error>,                 // errors
    document: Vec<String>,              // document tree
    document_fragment: Vec<String>,     // fragment
}

fn main () -> io::Result<()> {
    let default_dir = "./html5lib-tests";
    let dir = env::args().nth(1).unwrap_or(default_dir.to_string());

    let mut results = TestResults{
        tests: 0,
        assertions: 0,
        succeeded: 0,
        failed: 0,
        failed_position: 0,
    };

    for entry in fs::read_dir(dir + "/tree-construction")? {
        let entry = entry?;
        let path = entry.path();

        if ! path.ends_with("tests1.dat") {
            continue;
        }

        if !path.is_file() || path.extension().unwrap() != "dat" {
            continue;
        }

        let tests = read_tests(path.clone())?;
        println!("🏃‍♂️ Running {} tests from 🗄️ {:?}\n", tests.len(), path);

        let mut test_idx = 1;
        for test in tests {
            if test_idx == 23 {
                run_tree_test(test_idx, &test, &mut results);
            }

            test_idx += 1;
        }
    }

    println!("🏁 Tests completed: Ran {} tests, {} assertions, {} succeeded, {} failed ({} position failures)", results.tests, results.assertions, results.succeeded, results.failed, results.failed_position);
    Ok(())
}

fn read_tests(file_path: PathBuf) -> io::Result<Vec<Test>> {
    let file = File::open(file_path.clone())?;
    let reader = BufReader::new(file);

    let mut tests = Vec::new();
    let mut current_test = Test {
        file_path: file_path.to_str().unwrap().to_string(),
        line: 1,
        data: "".to_string(),
        errors: vec![],
        document: vec![],
        document_fragment: vec![],
    };
    let mut section: Option<&str> = None;

    for (line_num, line) in reader.lines().enumerate() {
        let line = line?;

        if line.starts_with("#data") {
            if !current_test.data.is_empty() || !current_test.errors.is_empty() || !current_test.document.is_empty() {
                current_test.data = current_test.data.trim_end().to_string();
                tests.push(current_test);
                current_test = Test{
                    file_path: file_path.to_str().unwrap().clone().to_string(),
                    line: line_num,
                    data: "".to_string(),
                    errors: vec![],
                    document: vec![],
                    document_fragment: vec![],
                };
            }
            section = Some("data");
        } else if line.starts_with('#') {
            section = match line.as_str() {
                "#errors" => Some("errors"),
                "#document" => Some("document"),
                _ => None,
            };
        } else if let Some(sec) = section {
            match sec {
                "data" => current_test.data.push_str(&line),
                "errors" => {
                    let re = Regex::new(r"\((?P<line>\d+),(?P<col>\d+)\): (?P<code>.+)").unwrap();
                    if let Some(caps) = re.captures(&line) {
                        let line = caps.name("line").unwrap().as_str().parse::<i64>().unwrap();
                        let col = caps.name("col").unwrap().as_str().parse::<i64>().unwrap();
                        let code = caps.name("code").unwrap().as_str().to_string();

                        current_test.errors.push(Error{ code, line, col });
                    }
                },
                "document" => current_test.document.push(line),
                "document_fragment" => current_test.document_fragment.push(line),
                _ => (),
            }
        }
    }

    // Push the last test if it has data
    if !current_test.data.is_empty() || !current_test.errors.is_empty() || !current_test.document.is_empty() {
        current_test.data = current_test.data.trim_end().to_string();
        tests.push(current_test);
    }

    Ok(tests)
}

fn run_tree_test(test_idx: usize,test: &Test, results: &mut TestResults) {
    println!("🧪 Running test #{}: {}::{}", test_idx, test.file_path, test.line);

    results.tests += 1;

    let old_failed = results.failed;

    let mut is = InputStream::new();
    is.read_from_str(test.data.as_str(), None);

    let mut parser = Html5Parser::new(&mut is);
    let (document, parse_errors) = parser.parse();

    // Check the document tree, which counts as a single assertion
    results.assertions += 1;
    if match_document_tree(document, &test.document) {
        results.succeeded += 1;
    } else {
        results.failed += 1;
    }

    if parse_errors.len() != test.errors.len() {
         println!("❌ Unexpected errors found (wanted {}, got {}): ", test.errors.len(), parse_errors.len());
    //     for want_err in &test.errors {
    //         println!("     * Want: '{}' at {}:{}", want_err.code, want_err.line, want_err.col);
    //     }
    //     for got_err in &parse_errors {
    //         println!("     * Got: '{}' at {}:{}", got_err.message, got_err.line, got_err.col);
    //     }
    //     results.assertions += 1;
    //     results.failed += 1;
    } else {
         println!("✅ Found {} errors", parse_errors.len());
    }
    //
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


    if old_failed != results.failed {
        println!("----------------------------------------");
        println!("📄 Input stream: ");
        println!("{}", test.data);
        println!("----------------------------------------");
        println!("🌳 Generated tree: ");
        println!("{}", document);
        println!("----------------------------------------");
        println!("🌳 Expected tree: ");
        for line in &test.document {
            println!("{}", line);
        }

        std::process::exit(1);
    }

    println!("----------------------------------------");
}

#[derive(PartialEq)]
enum ErrorResult {
    Success,            // Found the correct error
    Failure,            // Didn't find the error (not even with incorrect position)
    PositionFailure,    // Found the error, but on an incorrect position
}

#[derive(PartialEq)]
pub struct Error {
    pub code: String,
    pub line: i64,
    pub col: i64,
}

fn match_document_tree(document: &Document, expected: &Vec<String>) -> bool {
    match_node(0, -1, -1, document, expected).is_some()
}

fn match_node(node_idx: usize, expected_id: isize, indent: isize, document: &Document, expected: &Vec<String>) -> Option<usize> {
    let node = document.get_node_by_id(node_idx).unwrap();

    if node_idx > 0 {
        match &node.data {
            NodeData::Element { name, .. } => {
                let value = format!("|{}<{}>", " ".repeat((indent as usize * 2) + 1), name);
                if value != expected[expected_id  as usize] {
                    println!("❌ {}, Found unexpected element node: {}", expected[expected_id  as usize], name);
                    return None;
                } else {
                    println!("✅ {}", expected[expected_id  as usize]);
                }
            }
            NodeData::Text { value } => {
                let value = format!("|{}\"{}\"", " ".repeat(indent  as usize * 2 + 1), value);
                if value != expected[expected_id as usize] {
                    println!("❌ {}, Found unexpected text node: {}", expected[expected_id  as usize], value);
                    return None;
                } else {
                    println!("✅ {}", expected[expected_id  as usize]);
                }
            }
            _ => {}
        }
    }

    let mut next_expected_idx = expected_id + 1;

    for &child_idx in &node.children {
        if let Some(new_idx)  = match_node(child_idx, next_expected_idx, indent + 1, document, expected) {
            next_expected_idx = new_idx as isize;
        } else {
            return None;
        }
    }

    Some(next_expected_idx as usize)
}

#[allow(dead_code)]
fn match_error(got_err: &Error, expected_err: &Error) -> ErrorResult {
    if got_err == expected_err {
        // Found an exact match
        println!("✅ Found parse error '{}' at {}:{}", got_err.code, got_err.line, got_err.col);

        return ErrorResult::Success;
    }

    if got_err.code != expected_err.code {
        println!("❌ Expected error '{}' at {}:{}", expected_err.code, expected_err.line, expected_err.col);
        return ErrorResult::Failure;
    }

    // Found an error with the same code, but different line/pos
    println!("⚠️ Unexpected error position '{}' at {}:{} (got: {}:{})", expected_err.code, expected_err.line, expected_err.col, got_err.line, got_err.col);
    ErrorResult::PositionFailure
}