use gosub_engine::testing::tree_construction::fixture_from_filename;
use gosub_engine::testing::FIXTURE_ROOT;
use gosub_engine::types::Result;
use std::io::Write;
use std::path::Path;
use walkdir::WalkDir;

fn main() -> Result<()> {
    let files = get_files_from_path(format!("{}/tree-construction", FIXTURE_ROOT).as_str());

    let mut total = 0;
    let mut failed = 0;

    for file in files.iter() {
        let fixture = fixture_from_filename(file.as_str())?;
        print!("Test: ({:3}) {} [", fixture.tests.len(), file);
        let _ = std::io::stdout().flush();

        // Run tests
        for test in fixture.tests {
            let result = test.run().expect("problem running tree construction test");

            total += 1;
            if result.success() {
                print!(".");
            } else {
                print!("X");
                failed += 1;
            }
            let _ = std::io::stdout().flush();
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

fn get_files_from_path(root_dir: &str) -> Vec<String> {
    let mut files = Vec::new();

    for entry in WalkDir::new(root_dir)
        .follow_links(true)
        .into_iter()
        .flatten()
    {
        if entry.file_type().is_file() {
            if let Some(extension) = entry.path().extension() {
                if extension == "dat" {
                    if let Ok(relative_path) = entry
                        .path()
                        .strip_prefix(root_dir)
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
