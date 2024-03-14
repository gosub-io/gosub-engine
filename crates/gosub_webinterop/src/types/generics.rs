use crate::types::Primitive;
use quote::ToTokens;
use syn::Path;

#[derive(Clone, PartialEq, Debug)]
pub(crate) struct GenericProperty {
    pub(crate) param: Path,
    pub(crate) types: Vec<(Path, Primitive)>,
}

#[derive(Clone, PartialEq, Debug)]
pub(crate) struct Generics {
    pub(crate) matcher: GenericsMatcher,
    pub(crate) types: Vec<(Path, Primitive)>,
}

#[derive(Clone, PartialEq, Debug)]
pub(crate) enum GenericsMatcher {
    Param(Path),
    Trait(Path),
    Index(usize),
}

impl GenericsMatcher {
    pub(crate) fn is_match(&self, ty: &str, index: usize) -> bool {
        let ty = ty.replace(' ', "");
        match self {
            GenericsMatcher::Param(p) => p.to_token_stream().to_string().replace(' ', "") == ty,
            GenericsMatcher::Trait(p) => p.to_token_stream().to_string().replace(' ', "") == ty,
            GenericsMatcher::Index(i) => i == &index,
        }
    }

    pub(crate) fn new(generic: Path, func: &syn::ImplItemFn) -> GenericsMatcher {
        let mut generic_params = Vec::new();

        for generic in &func.sig.generics.params {
            if let syn::GenericParam::Type(t) = generic {
                generic_params.push(t.ident.clone());
            }
        }

        // check if it is a number
        if let Ok(a) = generic.to_token_stream().to_string().parse::<usize>() {
            return GenericsMatcher::Index(a);
        }

        if generic_params.contains(generic.get_ident().unwrap()) {
            return GenericsMatcher::Param(generic);
        }

        GenericsMatcher::Trait(generic)
    }

    pub(crate) fn get_matchers(
        generics: Vec<GenericProperty>,
        func: &syn::ImplItemFn,
    ) -> Vec<Generics> {
        let mut gen = Vec::new();

        for generic in generics {
            let matcher = GenericsMatcher::new(generic.param, func);

            if gen.iter().any(|g: &Generics| g.matcher == matcher) {
                panic!("Duplicate generic matcher");
            }

            gen.push(Generics {
                matcher,
                types: generic.types,
            });
        }

        gen
    }
}
