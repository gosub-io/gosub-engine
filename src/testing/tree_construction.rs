mod parser;

use self::parser::{ErrorSpec, ScriptMode, TestSpec, QUOTED_DOUBLE_NEWLINE};
use super::FIXTURE_ROOT;
use crate::html5::node::data::doctype::DocTypeData;
use crate::html5::node::{HTML_NAMESPACE, MATHML_NAMESPACE, SVG_NAMESPACE};
use crate::html5::parser::document::DocumentBuilder;
use crate::html5::parser::tree_builder::TreeBuilder;
use crate::html5::parser::Html5ParserOptions;
use crate::{
    html5::{
        input_stream::InputStream,
        node::{NodeData, NodeId},
        parser::{
            document::{Document, DocumentHandle},
            Html5Parser,
        },
    },
    types::{ParseError, Result},
};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Holds all tests as found in the given fixture file
#[derive(Debug, PartialEq)]
pub struct FixtureFile {
    pub tests: Vec<Test>,
    pub path: String,
}

/// Holds information about an error
#[derive(Clone, Debug, PartialEq)]
pub struct TestError {
    /// The code or message of the error
    pub code: String,
    /// The line number (1-based) where the error occurred
    pub line: i64,
    /// The column number (1-based) where the error occurred
    pub col: i64,
}

/// Holds a single parser test
#[derive(Debug, PartialEq)]
pub struct Test {
    /// Filename of the test
    pub file_path: String,
    /// Line number of the test
    pub line: usize,
    /// The specification of the test provided in the test file
    pub spec: TestSpec,
    /// The document tree that is expected to be parsed
    pub document: Vec<String>,
}

/// Holds the result of a single "node" (which is either an element, text or comment)
pub enum NodeResult {
    /// An attribute of an element node did not match
    AttributeMatchFailure {
        name: String,
        actual: String,
        expected: String,
    },

    /// The actual element does not match the expected element
    ElementMatchFailure {
        name: String,
        actual: String,
        expected: String,
    },

    /// The element matches the expected element
    ElementMatchSuccess {
        actual: String,
    },

    /// A text node did not match
    TextMatchFailure {
        actual: String,
        expected: String,
        text: String,
    },

    // A doctype node did not match
    DocTypeMatchFailure {
        actual: String,
        expected: String,
    },

    /// A comment node did not match
    CommentMatchFailure {
        actual: String,
        expected: String,
        comment: String,
    },

    /// A text node matches
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
    Success { actual: TestError },
    /// Didn't find the error (not even with incorrect position)
    Failure {
        actual: TestError,
        expected: TestError,
    },
    /// Found the error, but on an incorrect position
    PositionFailure {
        actual: TestError,
        expected: TestError,
    },
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
    pub fn data(&self) -> &str {
        self.spec.data.strip_suffix('\n').unwrap_or_default()
    }

    pub fn errors(&self) -> &Vec<ErrorSpec> {
        &self.spec.errors
    }

    /// Runs the test and returns the result
    pub fn run(&self) -> Result<Vec<TestResult>> {
        let mut results = vec![];

        for &scripting_enabled in self.script_modes() {
            let (actual_document, actual_errors) = self.parse(scripting_enabled)?;
            let root = self.match_document_tree(&actual_document.get());
            results.push(TestResult {
                root,
                actual_document,
                actual_errors,
            });
        }

        Ok(results)
    }

    /// Verifies that the tree construction code obtains the right result
    pub fn assert_valid(&self) {
        let results = self.run().expect("failed to parse");

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

                Some(NodeResult::DocTypeMatchFailure {
                    actual, expected, ..
                }) => {
                    panic!("doctype match failed, wanted: [{expected}], got: [{actual}]");
                }

                Some(NodeResult::ElementMatchFailure {
                    actual,
                    expected,
                    name,
                }) => {
                    panic!("element [{name}] match failed, wanted: [{expected}], got: [{actual}]");
                }

                Some(NodeResult::AttributeMatchFailure {
                    name,
                    actual,
                    expected,
                }) => {
                    panic!(
                        "attribute [{name}] match failed, wanted: [{expected}], got: [{actual}]"
                    );
                }

                Some(NodeResult::CommentMatchFailure {
                    actual, expected, ..
                }) => {
                    panic!("comment match failed, wanted: [{expected}], got: [{actual}]");
                }
            }

            tree.children.iter().for_each(assert_tree);
        }

        for result in results {
            assert_tree(&result.root);
            assert!(result.success(), "invalid tree-construction result");
        }
    }

    /// Run the parser and return the document and errors
    pub fn parse(&self, scripting_enabled: bool) -> Result<(DocumentHandle, Vec<ParseError>)> {
        // let mut is_fragment = false;
        let mut context_node = None;
        let document;

        let is_fragment;

        if let Some(fragment) = self.spec.document_fragment.clone() {
            // First, create a (fake) main document that contains only the fragment as node
            let main_document = DocumentBuilder::new_document();
            let mut main_document = Document::clone(&main_document);

            // Add context node
            let context_node_id = main_document.create_element(
                fragment.as_str(),
                NodeId::root(),
                None,
                HTML_NAMESPACE,
            );
            context_node = Some(
                main_document
                    .get()
                    .get_node_by_id(context_node_id)
                    .unwrap()
                    .clone(),
            );

            is_fragment = true;
            document = DocumentBuilder::new_document_fragment(context_node.clone().expect(""));
        } else {
            is_fragment = false;
            document = DocumentBuilder::new_document();
        };

        // Create a new parser
        let options = Html5ParserOptions { scripting_enabled };

        let mut is = InputStream::new();
        is.read_from_str(self.data(), None);

        let parse_errors = if is_fragment {
            Html5Parser::parse_fragment(
                &mut is,
                Document::clone(&document),
                &context_node.expect(""),
                Some(options),
            )?
        } else {
            Html5Parser::parse_document(&mut is, Document::clone(&document), Some(options))?
        };

        Ok((document, parse_errors))
    }

    /// Returns true if the whole document tree matches the expected result
    pub fn match_document_tree(&self, document: &Document) -> SubtreeResult {
        if self.spec.document_fragment.is_some() {
            self.match_node(NodeId::from(1), 0, 0, document)
        } else {
            self.match_node(NodeId::root(), 0, -1, document)
        }
    }

    /// Match a single node and its children
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
            NodeData::DocType(DocTypeData {
                name,
                pub_identifier,
                sys_identifier,
            }) => {
                let doctype_text = if pub_identifier.is_empty() && sys_identifier.is_empty() {
                    // <!DOCTYPE html>
                    name.to_string()
                } else {
                    // <!DOCTYPE html "pubid" "sysid">
                    format!(r#"{name} "{pub_identifier}" "{sys_identifier}""#,)
                };

                let actual = format!(
                    "|{}<!DOCTYPE {}>",
                    " ".repeat(indent as usize * 2 + 1),
                    doctype_text.trim(),
                );

                let expected = self.document[next_expected_idx as usize].to_owned();
                next_expected_idx += 1;

                if actual != expected {
                    let node = Some(NodeResult::DocTypeMatchFailure {
                        actual,
                        expected: "".to_string(),
                    });

                    return SubtreeResult {
                        node,
                        children: vec![],
                        next_expected_idx: None,
                    };
                }

                Some(NodeResult::TextMatchSuccess { expected })
            }
            NodeData::Element(element) => {
                let prefix: String = match &node.namespace {
                    Some(namespace) => match namespace.as_str() {
                        HTML_NAMESPACE => "".into(), // HTML elements don't have a prefix
                        SVG_NAMESPACE => "svg ".into(),
                        MATHML_NAMESPACE => "math ".into(),
                        _ => {
                            panic!("unknown namespace: {}", namespace);
                        }
                    },
                    None => "".into(),
                };

                let actual = format!(
                    "|{}<{}{}>",
                    " ".repeat((indent as usize * 2) + 1),
                    prefix,
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

                // Make sure the attributes are sorted
                let mut sorted_attrs = vec![];
                for attr in element.attributes.iter() {
                    sorted_attrs.push(attr);
                }
                sorted_attrs.sort_by(|a, b| a.0.cmp(b.0));

                for attr in sorted_attrs {
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

                // Text might be split over multiple lines, read all lines until we find the closing
                // quote.
                let mut expected = String::new();
                loop {
                    let tmp = self.document[next_expected_idx as usize].to_owned();
                    next_expected_idx += 1;

                    expected.push_str(&tmp);

                    if tmp.ends_with('\"') {
                        break;
                    } else {
                        // each line is terminated with a newline
                        expected.push('\n');
                    }
                }

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
    fn match_error(actual: &TestError, expected: &TestError) -> ErrorResult {
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

    pub fn script_modes(&self) -> &[bool] {
        match self.spec.script_mode {
            ScriptMode::ScriptOff => &[false],
            ScriptMode::ScriptOn => &[true],
            ScriptMode::Both => &[false, true],
        }
    }
}

pub fn fixture_from_filename(filename: &str) -> Result<FixtureFile> {
    let path = PathBuf::from(FIXTURE_ROOT)
        .join("tree-construction")
        .join(filename);
    fixture_from_path(&path)
}

// Split into an array of lines.  Combine lines in cases where a subsequent line does not
// have a "|" prefix using an "\n" delimiter.  Otherwise strip "\n" from lines.
fn document(s: &str) -> Vec<String> {
    let mut document = s
        .replace(QUOTED_DOUBLE_NEWLINE, "\"\n\n\"")
        .split('|')
        .skip(1)
        .filter_map(|l| {
            if l.is_empty() {
                None
            } else {
                Some(format!("|{}", l.trim_end()))
            }
        })
        .collect::<Vec<_>>();

    // TODO: drop the following line
    document.push("".into());
    document
}

/// Read a given test file and extract all test data
pub fn fixture_from_path(path: &PathBuf) -> Result<FixtureFile> {
    let input = fs::read_to_string(path)?;
    let path = path.to_string_lossy().into_owned();

    let tests = parser::parse_str(&input)?
        .into_iter()
        .map(|spec| Test {
            file_path: path.to_string(),
            line: spec.position.line,
            document: document(&spec.document),
            spec,
        })
        .collect::<Vec<_>>();

    Ok(FixtureFile {
        tests,
        path: path.to_string(),
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
