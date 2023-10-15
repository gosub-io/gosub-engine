use super::FIXTURE_ROOT;
use crate::types::Result;
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

#[derive(Debug, PartialEq)]
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

impl Test {
    // Check that the tree construction code doesn't panic
    pub fn run(&self) {
        // TODO: Fill this in later
    }

    // Verify that the tree construction code obtains the right result
    pub fn assert_valid(&self) {
        // TODO: Fill this in later
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
