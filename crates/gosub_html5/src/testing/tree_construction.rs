pub mod fixture;
mod generator;
pub mod parser;
pub mod result;

use crate::node::{HTML_NAMESPACE, MATHML_NAMESPACE, SVG_NAMESPACE};
use generator::TreeOutputGenerator;
use gosub_interface::config::{HasDocument, HasHtmlParser};
use gosub_interface::document::{Document, DocumentBuilder};

use gosub_interface::html5::{Html5Parser, ParserOptions};
use gosub_shared::byte_stream::{ByteStream, Config, Encoding, Location};
use gosub_shared::node::NodeId;
use gosub_shared::types::{ParseError, Result};
use parser::{ScriptMode, TestSpec};
use result::TestResult;
use result::{ResultStatus, TreeLineResult};
use std::collections::HashMap;

type ParseResult<T> = Result<(T, Vec<ParseError>)>;

/// Holds a single parser test
#[derive(Debug, Default, PartialEq, Clone)]
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
    #[must_use] 
    pub fn script_modes(&self) -> &[bool] {
        match self.spec.script_mode {
            ScriptMode::ScriptOff => &[false],
            ScriptMode::ScriptOn => &[true],
            ScriptMode::Both => &[false, true],
        }
    }

    #[must_use] 
    pub fn document_as_str(&self) -> &str {
        self.spec.document.as_str()
    }

    #[must_use] 
    pub fn spec_data(&self) -> &str {
        self.spec.data.as_str()
    }
}

/// Harness is a wrapper to run tree-construction tests
#[derive(Debug)]
pub struct Harness {
    // Test that is currently being run
    test: Test,
    /// Next line in the document array
    next_document_line: usize,
}

impl Default for Harness {
    fn default() -> Self {
        Self::new()
    }
}

impl Harness {
    /// Generated a new harness instance. It uses a dummy test that is replaced when `run_test` is called
    #[must_use]
    pub fn new() -> Self {
        Self {
            test: Test::default(),
            next_document_line: 0,
        }
    }

    /// Runs a single test and returns the test result of that run
    pub fn run_test<C: HasHtmlParser>(&mut self, test: Test, scripting_enabled: bool) -> Result<TestResult> {
        self.test = test;
        self.next_document_line = 0;

        let (actual_document, actual_errors) = self.do_parse::<C>(scripting_enabled)?;
        let result = self.generate_test_result::<C>(actual_document, &actual_errors);

        Ok(result)
    }

    /// Run the html5 parser and return the document tree and errors
    fn do_parse<C: HasHtmlParser>(&mut self, scripting_enabled: bool) -> ParseResult<C::Document> {
        let options = <<C::HtmlParser as Html5Parser<C>>::Options as ParserOptions>::new(scripting_enabled);
        let mut stream = ByteStream::new(
            Encoding::UTF8,
            Some(Config {
                cr_lf_as_one: true,
                replace_cr_as_lf: true,
                replace_high_ascii: false,
            }),
        );
        stream.read_from_str(self.test.spec_data(), None);
        stream.close();

        let (document, parse_errors) = if let Some(fragment) = self.test.spec.document_fragment.clone() {
            self.parse_fragment::<C>(fragment, stream, options, Location::default())?
        } else {
            let mut document = C::DocumentBuilder::new_document(None);
            let parser_errors = C::HtmlParser::parse(&mut stream, &mut document, Some(options))?;

            (document, parser_errors)
        };

        Ok((document, parse_errors))
    }

    fn parse_fragment<C: HasHtmlParser>(
        &mut self,
        fragment: String,
        mut stream: ByteStream,
        options: <C::HtmlParser as Html5Parser<C>>::Options,
        start_location: Location,
    ) -> ParseResult<C::Document> {
        // First, create a (fake) main document that contains only the fragment as node
        let mut main_doc_handle = C::DocumentBuilder::new_document(None);

        // let mut main_document = main_document.clone();
        let (element, namespace) = if fragment.starts_with("svg ") {
            (fragment.strip_prefix("svg ").unwrap().to_string(), SVG_NAMESPACE)
        } else if fragment.starts_with("math ") {
            (fragment.strip_prefix("math ").unwrap().to_string(), MATHML_NAMESPACE)
        } else {
            (fragment, HTML_NAMESPACE)
        };

        let quirks_mode = main_doc_handle.quirks_mode();

        // let doc_clone = DocumentHandle::clone(&main_document);
        // let mut doc = main_document.get_mut();

        let node = C::Document::new_element_node(element.as_str(), Some(namespace), HashMap::new(), start_location);

        let context_node_id = main_doc_handle.register_node_at(node, NodeId::root(), None);
        let context_node = main_doc_handle.node_by_id(context_node_id).unwrap();
        let _ = context_node_id;

        let mut document = C::DocumentBuilder::new_document_fragment(context_node, quirks_mode);

        let parser_errors = C::HtmlParser::parse_fragment(
            &mut stream,
            &mut document,
            context_node.clone(),
            Some(options),
            start_location,
        )?;

        Ok((document, parser_errors))
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
            let tmp = self.test.document[self.next_document_line].clone();
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

            // Only break if we're in a multi-line text, and we found the ending double-quote
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

    fn generate_test_result<C: HasDocument>(
        &mut self,
        document: C::Document,
        _parse_errors: &[ParseError],
    ) -> TestResult {
        let mut result = TestResult::default();

        let generator = TreeOutputGenerator::<C>::new(document);
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
        while let Some(expected_line) = self.next_line() {
            result.tree_results.push(TreeLineResult {
                index: line_idx,
                result: ResultStatus::Additional,
                expected: expected_line,
                actual: String::new(),
            });
            line_idx += 1;
        }

        result
    }
}
