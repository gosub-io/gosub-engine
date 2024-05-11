use crate::types::Primitive;
use quote::ToTokens;
use syn::Path;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GenericProperty {
    pub(crate) param: Path,
    pub(crate) types: Vec<(Path, Primitive)>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Generics {
    pub(crate) matcher: GenericsMatcher,
    pub(crate) types: Vec<(Path, Primitive)>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum GenericsMatcher {
    Param(Path),
    Trait(Path),
    Index(usize),
}

impl GenericsMatcher {
    pub(crate) fn is_match(&self, ty: &str, index: usize) -> bool {
        let ty = ty.replace(' ', "");
        match self {
            Self::Param(p) => p.to_token_stream().to_string().replace(' ', "") == ty,
            Self::Trait(p) => p.to_token_stream().to_string().replace(' ', "") == ty,
            Self::Index(i) => i == &index,
        }
    }

    pub(crate) fn new(generic: Path, func: &syn::ImplItemFn) -> Self {
        let mut generic_params = Vec::new();

        for generic in &func.sig.generics.params {
            if let syn::GenericParam::Type(t) = generic {
                generic_params.push(t.ident.clone());
            }
        }

        // check if it is a number
        if let Ok(a) = generic.to_token_stream().to_string().parse::<usize>() {
            return Self::Index(a);
        }

        if generic_params.contains(generic.get_ident().unwrap()) {
            return Self::Param(generic);
        }

        Self::Trait(generic)
    }

    pub(crate) fn get_matchers(
        generics: Vec<GenericProperty>,
        func: &syn::ImplItemFn,
    ) -> Vec<Generics> {
        let mut gen = Vec::new();

        for generic in generics {
            let matcher = Self::new(generic.param, func);

            assert!(!gen.iter().any(|g: &Generics| g.matcher == matcher), "Duplicate generic matcher");

            gen.push(Generics {
                matcher,
                types: generic.types,
            });
        }

        gen
    }
}
