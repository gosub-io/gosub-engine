use crate::testing::tree_construction::parser::{parse_fixture, QUOTED_DOUBLE_NEWLINE};
use crate::testing::tree_construction::Test;
use crate::testing::{FIXTURE_ROOT, TREE_CONSTRUCTION_PATH};
use gosub_shared::types::Result;
use std::fs;
use std::path::{Path, PathBuf};

/// Holds all tests as found in the given fixture file
#[derive(Debug, PartialEq)]
pub struct FixtureFile {
    /// All the tests extracted from this fixture file
    pub tests: Vec<Test>,
    /// Path to the fixture file
    pub path: String,
}

/// Reads a given test file and extract all test data
pub fn read_fixture_from_path(path: impl AsRef<Path>) -> Result<FixtureFile> {
    let input = fs::read_to_string(&path)?;
    let path = path.as_ref().to_string_lossy().into_owned();

    let tests = parse_fixture(&input)?
        .into_iter()
        .map(|spec| Test {
            file_path: path.clone(),
            line: spec.position.line,
            document: create_document_array(&spec.document),
            spec,
        })
        .collect::<Vec<_>>();

    Ok(FixtureFile { tests, path })
}

/// Returns true when the fixture at 'path' is a correct fixture file and is allowed to be used
/// according to the list of given filenames. If no filenames are given, all fixtures are used.
fn use_fixture(filenames: &[&str], path: impl AsRef<Path>) -> bool {
    if filenames.is_empty() {
        return true;
    }

    let path = path.as_ref();
    if !path.is_file() || path.extension().expect("file ending") != "dat" {
        return false;
    }

    filenames.iter().any(|filename| path.ends_with(filename))
}

/// Returns the root path for the fixtures
#[must_use]
pub fn fixture_root_path() -> PathBuf {
    PathBuf::from(FIXTURE_ROOT).join(TREE_CONSTRUCTION_PATH)
}

/// Read tree construction fixtures from the given path. If no filenames are given, all
/// fixtures are read, otherwise only the fixes with the given filenames are read.
pub fn read_fixtures(filenames: Option<&[&str]>) -> Result<Vec<FixtureFile>> {
    let filenames = filenames.unwrap_or_default();
    let mut files = vec![];

    for entry in fs::read_dir(fixture_root_path())? {
        let path = entry?.path();

        // Check if the fixture is a correct fixture file and if it's allowed to be used
        if !use_fixture(filenames, &path) {
            continue;
        }

        let file = read_fixture_from_path(&path)?;
        files.push(file);
    }

    Ok(files)
}

// Split a string into an array of lines.  Combine lines in cases where a subsequent line does not
// have a "|" prefix using an "\n" delimiter.  Otherwise strip "\n" from lines.
fn create_document_array(s: &str) -> Vec<String> {
    let document = s
        .replace(QUOTED_DOUBLE_NEWLINE, "\"\n\n\"")
        .split('|')
        .skip(1)
        .filter_map(|l| (!l.is_empty()).then(|| format!("|{}", l.trim_end())))
        .collect::<Vec<_>>();

    document
}
