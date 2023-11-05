pub mod fixture;
mod generator;
pub(crate) mod parser;
pub mod result;

use crate::html5::node::{HTML_NAMESPACE, MATHML_NAMESPACE, SVG_NAMESPACE};
use crate::html5::parser::document::DocumentBuilder;
use crate::html5::parser::tree_builder::TreeBuilder;
use crate::html5::parser::Html5ParserOptions;
use crate::testing::tree_construction::generator::TreeOutputGenerator;
use crate::testing::tree_construction::parser::{ScriptMode, TestSpec};
use crate::testing::tree_construction::result::{ResultStatus, TreeLineResult};
use crate::{
    bytes::CharIterator,
    html5::{
        node::NodeId,
        parser::{
            document::{Document, DocumentHandle},
            Html5Parser,
        },
    },
    types::{ParseError, Result},
};
use result::TestResult;

/// Holds a single parser test
#[derive(Debug, PartialEq, Clone)]
pub struct Test {
    /// Filename of the test
    pub file_path: String,
    /// Line number of the test
    pub line: usize,
    /// The specification of the test provided in the test file
    pub spec: TestSpec,
    /// The document tree as found in the spec converted to an array
    pub document: Vec<String>,
}

impl Test {
    /// Returns the script modes that should be tested as an array
    pub fn script_modes(&self) -> &[bool] {
        match self.spec.script_mode {
            ScriptMode::ScriptOff => &[false],
            ScriptMode::ScriptOn => &[true],
            ScriptMode::Both => &[false, true],
        }
    }

    pub fn document_as_str(&self) -> &str {
        return self.spec.document.as_str();
    }
}

impl Default for Test {
    fn default() -> Self {
        Self {
            file_path: "".to_string(),
            line: 0,
            spec: TestSpec::default(),
            document: vec![],
        }
    }
}

/// Harness is a wrapper to run tree-construction tests
#[derive(Default)]
pub struct Harness {
    // Test that is currently being run
    test: Test,
    /// Next line in the document array
    next_document_line: usize,
}

impl Harness {
    /// Generated a new harness instance. It uses a dummy test that is replaced when run_test is called
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs a single test and returns the test result of that run
    pub fn run_test(&mut self, test: Test, scripting_enabled: bool) -> Result<TestResult> {
        self.test = test;
        self.next_document_line = 0;

        let (actual_document, actual_errors) = self.do_parse(scripting_enabled)?;
        let result = self.generate_test_result(Document::clone(&actual_document), &actual_errors);

        Ok(result)
    }

    /// Run the html5 parser and return the document tree and errors
    fn do_parse(&mut self, scripting_enabled: bool) -> Result<(DocumentHandle, Vec<ParseError>)> {
        let mut context_node = None;
        let document;
        let is_fragment;

        if let Some(fragment) = self.test.spec.document_fragment.clone() {
            // First, create a (fake) main document that contains only the fragment as node
            let main_document = DocumentBuilder::new_document();
            let mut main_document = Document::clone(&main_document);
            let (element, namespace) = if fragment.starts_with("svg ") {
                (
                    fragment.strip_prefix("svg ").unwrap().to_string(),
                    SVG_NAMESPACE,
                )
            } else if fragment.starts_with("math ") {
                (
                    fragment.strip_prefix("math ").unwrap().to_string(),
                    MATHML_NAMESPACE,
                )
            } else {
                (fragment, HTML_NAMESPACE)
            };

            // Add context node
            let context_node_id =
                main_document.create_element(element.as_str(), NodeId::root(), None, namespace);
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

        let options = Html5ParserOptions { scripting_enabled };

        let mut chars = CharIterator::new();
        chars.read_from_str(self.test.spec.data.as_str(), None);

        let parse_errors = if is_fragment {
            Html5Parser::parse_fragment(
                &mut chars,
                Document::clone(&document),
                &context_node.expect(""),
                Some(options),
            )?
        } else {
            Html5Parser::parse_document(&mut chars, Document::clone(&document), Some(options))?
        };

        Ok((document, parse_errors))
    }

    /// Retrieves the next line from the spec document
    fn next_line(&mut self) -> Option<String> {
        let mut line = String::new();
        let mut is_multi_line_text = false;

        loop {
            // If we are at the end of the document, we return None
            if self.next_document_line >= self.test.document.len() {
                return None;
            }

            // Get the next line
            let tmp = self.test.document[self.next_document_line].to_owned();
            self.next_document_line += 1;

            // If we have a starting quote, but not an ending quote, we are a multi-line text
            if tmp.starts_with('\"') && !tmp.ends_with('\"') {
                is_multi_line_text = true;
            }

            // Add the line to the current line if we are a multiline
            if is_multi_line_text {
                line.push_str(&tmp);
            } else {
                line = tmp;
            }

            // Only break if we're in a multi-line text and we found the ending double-quote
            if is_multi_line_text && line.ends_with('\"') {
                break;
            }

            // if we are not a multi-line, we can just break
            if !is_multi_line_text {
                break;
            }

            // Otherwise we continue with the next line (multi-line text)
        }

        Some(line.to_string())
    }

    fn generate_test_result(
        &mut self,
        document: DocumentHandle,
        _parse_errors: &[ParseError],
    ) -> TestResult {
        let mut result = TestResult::default();

        let generator = TreeOutputGenerator::new(document);
        let actual = generator.generate();

        let mut line_idx = 1;
        for actual_line in actual {
            let mut status = ResultStatus::Success;

            let expected_line = self.next_line();
            match expected_line.clone() {
                Some(expected_line) => {
                    if actual_line != expected_line {
                        status = ResultStatus::Mismatch;
                    }
                }
                None => {
                    status = ResultStatus::Missing;
                }
            }

            result.tree_results.push(TreeLineResult {
                index: line_idx,
                result: status,
                expected: expected_line.unwrap_or_default().to_string(),
                actual: actual_line.to_string(),
            });
            line_idx += 1;
        }

        // Check if we have additional lines and if so, add as errors
        loop {
            let expected_line = self.next_line();
            if expected_line.is_none() {
                break;
            }

            result.tree_results.push(TreeLineResult {
                index: line_idx,
                result: ResultStatus::Additional,
                expected: expected_line.expect("").to_string(),
                actual: "".into(),
            });
            line_idx += 1;
        }

        // if ! result.is_success() {
        //     let actual = generator.generate();
        //     let expected = self.test.document.clone();
        //     println!("\n\nactual   : {:?}", actual);
        //     println!("expected : {:?}\n\n", expected);
        // }

        result
    }
}

/*
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
            assert_tree(&result.result);
            assert!(result.success(), "invalid tree-construction result");
        }
    }


    /// Returns true if the whole document tree matches the expected result
    fn match_document_tree(&self, document: &Document) -> SubtreeResult {
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

                let expected = self.get_next_line().expect("line");
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

                let expected = self.get_next_line().expect("line");

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
                    let expected = self.get_next_line().expect("line");

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

                let expected = self.get_next_line().expect("line");
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
                let expected = self.get_next_line().expect("line");

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
}
 */
