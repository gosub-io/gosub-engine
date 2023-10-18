use super::FIXTURE_ROOT;
use crate::{
    html5_parser::{
        error_logger::ParseError,
        input_stream::InputStream,
        node::{NodeData, NodeId},
        parser::{
            document::{Document, DocumentHandle},
            Html5Parser,
        },
    },
    types::Result,
};
use regex::Regex;
use std::{
    fs::{self, File},
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};

#[derive(Debug, PartialEq)]
pub struct FixtureFile {
    pub tests: Vec<Test>,
    pub path: PathBuf,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Error {
    pub code: String,
    pub line: i64,
    pub col: i64,
}

#[derive(Debug, PartialEq)]
pub struct Test {
    /// Filename of the test
    pub file_path: String,
    /// Line number of the test
    pub line: usize,
    /// input stream
    pub data: String,
    /// errors
    pub errors: Vec<Error>,
    /// document tree
    pub document: Vec<String>,
    /// fragment
    document_fragment: Vec<String>,
}

pub enum NodeResult {
    AttributeMatchFailure {
        name: String,
        actual: String,
        expected: String,
    },

    ElementMatchFailure {
        name: String,
        actual: String,
        expected: String,
    },

    ElementMatchSuccess {
        actual: String,
    },

    TextMatchFailure {
        actual: String,
        expected: String,
        text: String,
    },

    CommentMatchFailure {
        actual: String,
        expected: String,
        comment: String,
    },

    TextMatchSuccess {
        expected: String,
    },
}

pub struct SubtreeResult {
    pub node: Option<NodeResult>,
    pub children: Vec<SubtreeResult>,
    next_expected_idx: Option<usize>,
}

impl SubtreeResult {
    pub fn valid(&self) -> bool {
        self.next_expected_idx.is_some()
    }
}

#[derive(PartialEq)]
pub enum ErrorResult {
    /// Found the correct error
    Success { actual: Error },
    /// Didn't find the error (not even with incorrect position)
    Failure { actual: Error, expected: Error },
    /// Found the error, but on an incorrect position
    PositionFailure { actual: Error, expected: Error },
}

pub struct TestResult {
    pub root: SubtreeResult,
    pub actual_document: DocumentHandle,
    pub actual_errors: Vec<ParseError>,
}

impl TestResult {
    pub fn success(&self) -> bool {
        self.root.valid()
    }
}

impl Test {
    // Check that the tree construction code doesn't panic
    pub fn run(&self) -> Result<TestResult> {
        let (actual_document, actual_errors) = self.parse()?;
        let root = self.match_document_tree(&actual_document.get());

        Ok(TestResult {
            root,
            actual_document,
            actual_errors,
        })
    }

    // Verify that the tree construction code obtains the right result
    pub fn assert_valid(&self) {
        let result = self.run().expect("failed to parse");

        fn assert_tree(tree: &SubtreeResult) {
            match &tree.node {
                Some(NodeResult::ElementMatchSuccess { .. })
                | Some(NodeResult::TextMatchSuccess { .. })
                | None => {}

                Some(NodeResult::TextMatchFailure {
                    actual, expected, ..
                }) => {
                    panic!("text match failed, wanted: [{expected}], got: [{actual}]");
                }

                Some(NodeResult::ElementMatchFailure {
                    actual,
                    expected,
                    name,
                }) => {
                    panic!("element [{name}] match failed, wanted: [{expected}], got: [{actual}]");
                }
            }

            tree.children.iter().for_each(assert_tree);
        }

        assert_tree(&result.root);
        assert!(result.success(), "invalid tree-construction result");
    }

    pub fn parse(&self) -> Result<(DocumentHandle, Vec<ParseError>)> {
        // Do the actual parsing
        let mut is = InputStream::new();
        is.read_from_str(self.data.as_str(), None);

        let mut parser = Html5Parser::new(&mut is);
        let document = Document::shared();
        let parse_errors = parser.parse(Document::clone(&document))?;

        Ok((document, parse_errors))
    }

    fn match_document_tree(&self, document: &Document) -> SubtreeResult {
        self.match_node(NodeId::root(), 0, -1, document)
    }

    fn match_node(
        &self,
        node_idx: NodeId,
        document_offset_id: isize,
        indent: isize,
        document: &Document,
    ) -> SubtreeResult {
        let mut next_expected_idx = document_offset_id;

        let node = document.get_node_by_id(node_idx).unwrap();

        let node_result = match &node.data {
            NodeData::Element(element) => {
                let actual = format!(
                    "|{}<{}>",
                    " ".repeat((indent as usize * 2) + 1),
                    element.name()
                );
                let expected = self.document[next_expected_idx as usize].to_owned();
                next_expected_idx += 1;

                if actual != expected {
                    let node = Some(NodeResult::ElementMatchFailure {
                        name: element.name().to_owned(),
                        actual,
                        expected,
                    });

                    return SubtreeResult {
                        node,
                        children: vec![],
                        next_expected_idx: None,
                    };
                }

                // Check attributes if any
                for attr in element.attributes.iter() {
                    let expected = self.document[next_expected_idx as usize].to_owned();
                    next_expected_idx += 1;

                    let actual = format!(
                        "|{}{}=\"{}\"",
                        " ".repeat((indent as usize * 2) + 3),
                        attr.0,
                        attr.1
                    );

                    if actual != expected {
                        let node = Some(NodeResult::AttributeMatchFailure {
                            name: element.name().to_owned(),
                            actual,
                            expected,
                        });

                        return SubtreeResult {
                            node,
                            children: vec![],
                            next_expected_idx: None,
                        };
                    }
                }

                Some(NodeResult::ElementMatchSuccess { actual })
            }

            NodeData::Text(text) => {
                let actual = format!(
                    "|{}\"{}\"",
                    " ".repeat(indent as usize * 2 + 1),
                    text.value()
                );
                let expected = self.document[next_expected_idx as usize].to_owned();
                next_expected_idx += 1;

                if actual != expected {
                    let node = Some(NodeResult::TextMatchFailure {
                        actual,
                        expected,
                        text: text.value().to_owned(),
                    });

                    return SubtreeResult {
                        node,
                        children: vec![],
                        next_expected_idx: None,
                    };
                }

                Some(NodeResult::TextMatchSuccess { expected })
            }

            NodeData::Comment(comment) => {
                let actual = format!(
                    "|{}<!-- {} -->",
                    " ".repeat(indent as usize * 2 + 1),
                    comment.value()
                );
                let expected = self.document[next_expected_idx as usize].to_owned();
                next_expected_idx += 1;

                if actual != expected {
                    let node = Some(NodeResult::CommentMatchFailure {
                        actual,
                        expected,
                        comment: comment.value().to_owned(),
                    });

                    return SubtreeResult {
                        node,
                        children: vec![],
                        next_expected_idx: None,
                    };
                }

                Some(NodeResult::TextMatchSuccess { expected })

            }
            _ => None,
        };

        let mut children = vec![];

        for &child_idx in &node.children {
            let child_result = self.match_node(child_idx, next_expected_idx, indent + 1, document);
            let next_id = child_result.next_expected_idx;
            children.push(child_result);

            if let Some(next_id) = next_id {
                next_expected_idx = next_id as isize;
                continue;
            }

            // Child node didn't match, exit early with what we have
            return SubtreeResult {
                node: node_result,
                next_expected_idx: None,
                children,
            };
        }

        SubtreeResult {
            node: node_result,
            children,
            next_expected_idx: Some(next_expected_idx as usize),
        }
    }

    #[allow(dead_code)]
    fn match_error(actual: &Error, expected: &Error) -> ErrorResult {
        if actual == expected {
            return ErrorResult::Success {
                actual: actual.to_owned(),
            };
        }

        if actual.code != expected.code {
            return ErrorResult::Failure {
                actual: actual.to_owned(),
                expected: expected.to_owned(),
            };
        }

        ErrorResult::PositionFailure {
            expected: expected.to_owned(),
            actual: actual.to_owned(),
        }
    }
}

pub fn fixture_from_filename(filename: &str) -> Result<FixtureFile> {
    let path = PathBuf::from(FIXTURE_ROOT)
        .join("tree-construction")
        .join(filename);
    fixture_from_path(&path)
}

/// Read given tests file and extract all test data
pub fn fixture_from_path(path: &PathBuf) -> Result<FixtureFile> {
    let file = File::open(path)?;
    // TODO: use thiserror to translate library errors
    let reader = BufReader::new(file);

    let mut tests = Vec::new();
    let mut current_test = Test {
        file_path: path.to_str().unwrap().to_string(),
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
            if !current_test.data.is_empty()
                || !current_test.errors.is_empty()
                || !current_test.document.is_empty()
            {
                current_test.data = current_test.data.trim_end().to_string();
                tests.push(current_test);
                current_test = Test {
                    file_path: path.to_str().unwrap().to_string(),
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

                        current_test.errors.push(Error { code, line, col });
                    }
                }
                "document" => current_test.document.push(line),
                "document_fragment" => current_test.document_fragment.push(line),
                _ => (),
            }
        }
    }

    // Push the last test if it has data
    if !current_test.data.is_empty()
        || !current_test.errors.is_empty()
        || !current_test.document.is_empty()
    {
        current_test.data = current_test.data.trim_end().to_string();
        tests.push(current_test);
    }

    Ok(FixtureFile {
        tests,
        path: path.to_path_buf(),
    })
}

fn use_fixture(filenames: &[&str], path: &Path) -> bool {
    if !path.is_file() || path.extension().expect("file ending") != "dat" {
        return false;
    }

    if filenames.is_empty() {
        return true;
    }

    filenames.iter().any(|filename| path.ends_with(filename))
}

pub fn fixtures(filenames: Option<&[&str]>) -> Result<Vec<FixtureFile>> {
    let root = PathBuf::from(FIXTURE_ROOT).join("tree-construction");
    let filenames = filenames.unwrap_or_default();
    let mut files = vec![];

    for entry in fs::read_dir(root)? {
        let path = entry?.path();

        if !use_fixture(filenames, &path) {
            continue;
        }

        let file = fixture_from_path(&path)?;
        files.push(file);
    }

    Ok(files)
}
