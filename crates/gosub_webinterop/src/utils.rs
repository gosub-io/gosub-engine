use std::env;

use syn::Path;

#[allow(dead_code)]
pub fn crate_ident() -> Path {
    let mut name = env::var("CARGO_PKG_NAME").unwrap();
    if name == "gosub_webexecutor" {
        name = "crate".to_string();
    }

    let name = name.replace('-', "_");

    syn::parse_str::<Path>(&name).unwrap()
}

pub fn crate_name() -> String {
    env::var("CARGO_PKG_NAME").unwrap()
}
