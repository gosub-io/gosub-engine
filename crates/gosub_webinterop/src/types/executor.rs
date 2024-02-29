#[derive(PartialEq, Debug, Clone)]
#[allow(clippy::upper_case_acronyms)]
pub(crate) enum Executor {
    JS,
    WASM,
    Both,
    None,
}

impl Executor {
    pub(crate) fn is_js(&self) -> bool {
        *self == Executor::JS || *self == Executor::Both
    }

    #[allow(dead_code)] // needed when we implement WASM
    pub(crate) fn is_wasm(&self) -> bool {
        *self == Executor::WASM || *self == Executor::Both
    }
}
