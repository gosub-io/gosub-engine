#[derive(PartialEq, Eq, Debug, Clone)]
#[allow(clippy::upper_case_acronyms)]
pub enum Executor {
    JS,
    WASM,
    Both,
    None,
}

impl Executor {
    pub(crate) fn is_js(&self) -> bool {
        *self == Self::JS || *self == Self::Both
    }

    #[allow(dead_code)] // needed when we implement WASM
    pub(crate) fn is_wasm(&self) -> bool {
        *self == Self::WASM || *self == Self::Both
    }
}
