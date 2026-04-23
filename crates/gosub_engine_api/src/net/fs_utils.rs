use std::io;
use std::path::Path;
use tempfile::{Builder, NamedTempFile};
use url::Url;

/// Create a temp file in the same directory as `dest` for atomic renaming
/// Example: `/downloads/file.pdf` -> `/downloads/.file.pdf.part-AB12cd.tmp`
pub fn temp_path_for(dest: &Path) -> io::Result<NamedTempFile> {
    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    let filename = dest.file_name().unwrap_or_default();

    // Create a temp file with a prefix pattern that's less likely to conflict
    Builder::new()
        .prefix(&format!(".{}.part-", filename.to_string_lossy()))
        .suffix(".tmp")
        .tempfile_in(parent)
}

/// Create a temp staging file in the system temp dir for OpenExternal flows
/// Tries to derive a friendly name from the URL path
/// Example: `https://example.com/a/b/video.mp4` -> `/tmp/video.mp4.tmp-AB12cd`
#[allow(unused)]
pub fn stage_temp_path_for(url: &Url) -> io::Result<NamedTempFile> {
    let base_name = url
        .path_segments()
        .and_then(|mut it| it.next_back())
        .unwrap_or("download");

    let sanitized_base = sanitize_filename(base_name);
    let prefix = if sanitized_base.is_empty() {
        "download".to_string()
    } else {
        sanitized_base
    };

    // Create temp file with appropriate prefix
    Builder::new()
        .prefix(&format!("{}.tmp-", prefix))
        .tempfile()
}

#[allow(unused)]
fn sanitize_filename(s: &str) -> String {
    let mut result: String = s
        .chars()
        .map(|c| {
            // Allow simple safe set; replace everything else with '_'
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();

    // Remove leading/trailing non-alphanumeric characters and dots
    result = result
        .trim_matches(|c: char| !c.is_ascii_alphanumeric() || c == '.')
        .to_string();

    // Remove any remaining consecutive special characters
    let mut prev_was_special = false;
    result = result
        .chars()
        .filter(|&c| {
            let is_special = !c.is_ascii_alphanumeric();
            let keep = !is_special || !prev_was_special;
            prev_was_special = is_special;
            keep
        })
        .collect();

    // Ensure filename is not empty, too short, or problematic
    if result.is_empty() || result == "." || result == ".." {
        "download".to_string()
    } else {
        // Reasonable filename length limit
        result.chars().take(200).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_temp_path_creation() -> io::Result<()> {
        let dest = Path::new("/tmp/testfile.txt");
        let temp_file = temp_path_for(dest)?;

        assert!(temp_file.path().exists());
        assert!(temp_file.path().parent() == Some(Path::new("/tmp")));
        assert!(temp_file
            .path()
            .to_string_lossy()
            .contains("testfile.txt.part-"));

        Ok(())
    }

    #[test]
    fn test_stage_temp_path_creation() -> io::Result<()> {
        let url = Url::parse("https://example.com/path/to/file.pdf").unwrap();
        let temp_file = stage_temp_path_for(&url)?;

        assert!(temp_file.path().exists());
        assert!(temp_file.path().parent() == Some(std::env::temp_dir().as_path()));
        assert!(temp_file.path().to_string_lossy().contains("file.pdf.tmp-"));

        Ok(())
    }

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(sanitize_filename("normal file.txt"), "normal file.txt");
        assert_eq!(sanitize_filename("file with/slashes"), "file with_slashes");
        assert_eq!(
            sanitize_filename("file with\\backslashes"),
            "file with_backslashes"
        );
        assert_eq!(sanitize_filename("file with..dots"), "file with.dots");
        assert_eq!(sanitize_filename(""), "download");
        assert_eq!(sanitize_filename("."), "download");
        assert_eq!(sanitize_filename(".."), "download");
        assert_eq!(
            sanitize_filename("very<>long|filename*with?many:bad\"chars"),
            "very_long_filename_with_many_bad_chars"
        );
    }
}
