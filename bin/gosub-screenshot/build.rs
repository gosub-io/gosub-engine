fn main() {
    // Git SHA (short)
    let sha = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=BUILD_GIT_SHA={sha}");

    // Build date/time (UTC)
    let date = std::process::Command::new("date")
        .args(["-u", "+%Y-%m-%d %H:%M UTC"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=BUILD_DATE={date}");

    // Re-run when HEAD changes (new commit or branch switch)
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let git_head = std::path::Path::new(&manifest_dir).join("../../.git/HEAD");
        if git_head.exists() {
            println!("cargo:rerun-if-changed={}", git_head.display());
        }
    }
}
