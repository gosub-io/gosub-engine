use crate::types::parse_impl;
use proc_macro2::TokenStream;
use quote::{quote, ToTokens};
use syn::{Expr, Path};

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Reference {
    None,
    Ref,
    MutRef,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Type {
    pub(crate) reference: Reference,
    pub(crate) ty: TypeT,
}

#[derive(Clone, PartialEq, Debug)]
pub enum TypeT {
    Type(Path),
    Array(Box<Type>, Option<Expr>),
    Tuple(Vec<Type>), //Array on the JS side
    Generic(Vec<Path>),
}

impl Type {
    pub(crate) fn parse(ty: &syn::Type, allow_ref: bool) -> Result<Self, &'static str> {
        match ty {
            syn::Type::Reference(r) => {
                if !allow_ref {
                    return Err("type can't be a reference here");
                }

                Ok(Self {
                    reference: if r.mutability.is_none() {
                        Reference::Ref
                    } else {
                        Reference::MutRef
                    },
                    ty: Self::parse(&r.elem, false)
                        .map_err(|_| "double references not supported!")?
                        .ty,
                })
            }
            syn::Type::Array(a) => Ok(Self {
                reference: Reference::None,
                ty: TypeT::Array(
                    Box::new(Self::parse(&a.elem, allow_ref)?),
                    Some(a.len.clone()),
                ),
            }),
            syn::Type::Slice(s) => Ok(Self {
                reference: Reference::None,
                ty: TypeT::Array(Box::new(Self::parse(&s.elem, allow_ref)?), None),
            }),
            syn::Type::Tuple(t) => {
                let mut elements = Vec::with_capacity(t.elems.len());

                for elem in &t.elems {
                    elements.push(Self::parse(elem, allow_ref)?);
                }

                Ok(Self {
                    reference: Reference::None,
                    ty: TypeT::Tuple(elements),
                })
            }

            syn::Type::Path(p) => Ok(Self {
                reference: Reference::None,
                ty: TypeT::Type(p.path.clone()),
            }),

            syn::Type::ImplTrait(p) => Ok(Self {
                reference: Reference::None,
                ty: TypeT::Generic(parse_impl(&p.bounds)?),
            }),

            t => {
                panic!("Invalid argument type: {}", t.into_token_stream());
            }
        }
    }

    pub(crate) fn mutability(&self, slices: bool) -> TokenStream {
        if self.reference == Reference::MutRef && (!matches!(self.ty, TypeT::Array(..)) || slices) {
            quote! { mut }
        } else {
            TokenStream::new()
        }
    }

    pub(crate) fn get_reference(&self) -> TokenStream {
        match self.reference {
            Reference::Ref => quote! { & },
            Reference::MutRef => quote! { &mut },
            Reference::None => TokenStream::new(),
        }
    }

    pub(crate) fn ty(&self) -> TokenStream {
        match &self.ty {
            TypeT::Type(_) | TypeT::Generic(_) => quote! { _ },
            TypeT::Array(t, _) => {
                let ty = t.ty();
                quote! { Vec<#ty> }
            }
            TypeT::Tuple(t) => {
                let mut types = Vec::new();
                for t in t {
                    types.push(t.ty());
                }

                quote! { (#(#types),*) }
            }
        }
    }
}

impl TypeT {
    pub(crate) fn generics(&self) -> Option<String> {
        Some(match self {
            Self::Type(p) => p.get_ident()?.to_token_stream().to_string(),
            Self::Generic(t) => {
                let mut out = String::new();
                for (i, p) in t.iter().enumerate() {
                    if i != 0 {
                        out.push('+');
                    }
                    out.push_str(&p.get_ident()?.to_token_stream().to_string());
                }
                out
            }
            _ => None?,
        })
    }
}
