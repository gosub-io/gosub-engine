use gosub_shared::types::Result;
use gosub_testing::testing::tree_construction::fixture::{
    fixture_root_path, read_fixture_from_path,
};
use gosub_testing::testing::tree_construction::Harness;
use std::io::Write;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

fn main() -> Result<()> {
    let mut files = get_files_from_path(fixture_root_path());
    files.sort();

    let mut total = 0;
    let mut failed = 0;

    for file in &files {
        // if file != "math.dat" {
        //     continue;
        // }
        let fixture = read_fixture_from_path(fixture_root_path().join(file))?;

        print!("Test: ({:3}) {} [", fixture.tests.len(), file);
        let _ = std::io::stdout().flush();

        let mut harness = Harness::new();

        // Run tests
        for test in &fixture.tests {
            for &scripting_enabled in test.script_modes() {
                let result = harness
                    .run_test(test.clone(), scripting_enabled)
                    .expect("problem parsing");

                total += 1;

                if result.is_success() {
                    print!(".");
                } else {
                    print!("X");
                    failed += 1;
                }
                let _ = std::io::stdout().flush();
            }
        }

        println!("]");
    }

    println!(
        "All tests completed. {}/{} ({:.2}%) passed.",
        total - failed,
        total,
        (total - failed) as f32 / total as f32 * 100_f32
    );

    Ok(())
}

fn get_files_from_path(dir: PathBuf) -> Vec<String> {
    let mut files = Vec::new();

    for entry in WalkDir::new(dir.clone())
        .follow_links(true)
        .into_iter()
        .flatten()
    {
        if entry.file_type().is_file() {
            if let Some(extension) = entry.path().extension() {
                if extension == "dat" {
                    if let Ok(relative_path) = entry
                        .path()
                        .strip_prefix(dir.clone())
                        .map(Path::to_str)
                        .map(|s| s.unwrap().to_string())
                    {
                        files.push(relative_path);
                    }
                }
            }
        }
    }

    files
}
