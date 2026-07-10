use cow_utils::CowUtils;
use std::env;
use syn::Path;

#[allow(dead_code)]
pub fn crate_ident() -> Path {
    // CARGO_PKG_NAME is always set by Cargo while a proc-macro is expanded.
    let mut name = env::var("CARGO_PKG_NAME").expect("CARGO_PKG_NAME is always set by Cargo");
    if name == "gosub_webexecutor" {
        name = "crate".to_string();
    }

    let name = name.cow_replace('-', "_");

    // A sanitized crate name (`-` -> `_`) is always a valid path identifier.
    syn::parse_str::<Path>(&name).expect("sanitized crate name is a valid path")
}

pub fn crate_name() -> String {
    // CARGO_PKG_NAME is always set by Cargo while a proc-macro is expanded.
    env::var("CARGO_PKG_NAME").expect("CARGO_PKG_NAME is always set by Cargo")
}
