use proc_macro2::TokenStream;
use quote::{format_ident, quote, ToTokens};
use syn::punctuated::Punctuated;
use syn::{Path, Token, TypeParamBound};

use crate::types::{handle_slice_conv, Generics, Reference, Type, TypeT};

#[derive(Clone, PartialEq, Debug)]
pub struct Arg {
    pub(crate) index: usize,
    pub(crate) ty: Type,
    pub(crate) variant: ArgVariant,
}

impl Arg {
    pub(crate) fn prepare(&self) -> TokenStream {
        let index = self.index;
        let arg_name = format_ident!("arg{}", index);

        let rust_type = &self.ty.ty();

        let conv = handle_slice_conv(&self.ty, &arg_name);

        let mutability = &self
            .ty
            .mutability(conv.to_string() == TokenStream::new().to_string());

        let get_args = if index == 0 {
            quote! {
                args.variadic(ctx.clone())
            }
        } else {
            quote! {
                args.variadic_start(#index, ctx.clone())
            }
        };

        match self.variant {
            ArgVariant::Variadic => {
                quote! {
                    let #mutability #arg_name = #get_args;
                }
            }
            ArgVariant::Generic => {
                quote! {
                    let Some(#arg_name) = args.get(#index, ctx.clone()) else {
                        cb.error("failed to get argument");
                        return;
                    };
                }
            }
            ArgVariant::Normal => {
                quote! {
                    let Some(#arg_name) = args.get(#index, ctx.clone()) else {
                        cb.error("failed to get argument");
                        return;
                    };

                    let Ok(#mutability #arg_name): Result<#rust_type> = #arg_name.to_rust_value() else {
                        cb.error("failed to convert argument");
                        return;
                    };

                    #conv
                }
            }

            _ => TokenStream::new(),
        }
    }

    pub(crate) fn call(&self, index: usize) -> TokenStream {
        let arg_name = format_ident!("arg{}", index);
        if self.variant == ArgVariant::Context {
            return match self.ty.reference {
                Reference::Ref => quote! { &ctx },
                Reference::MutRef => panic!("Context argument cannot be referenced mutable"),
                Reference::None => quote! { ctx.clone() },
            };
        }

        let reference = &self.ty.get_reference();

        quote! { #reference #arg_name }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ArgVariant {
    Normal,
    Variadic,
    Context,
    Generic,
}

impl Arg {
    pub(crate) fn parse(
        arg: &syn::Type,
        index: usize,
        generics: &[Generics],
    ) -> Result<Self, &'static str> {
        let ty = Type::parse(arg, true)?;
        let mut variant = ArgVariant::Normal;

        match &ty.ty {
            TypeT::Type(p) => {
                if let Some(s) = p.segments.last() {
                    if s.ident == "Context" {
                        variant = ArgVariant::Context;
                    } else if s.ident == "VariadicArgs" {
                        variant = ArgVariant::Variadic;
                    }
                    if generics.iter().any(|gen| {
                        gen.matcher
                            .is_match(&p.to_token_stream().to_string(), index)
                    }) {
                        variant = ArgVariant::Generic;
                    }
                }
            }
            TypeT::Generic(p) => {
                for p in p {
                    if let Some(s) = p.segments.last() {
                        if s.ident == "JSContext" {
                            variant = ArgVariant::Context;
                        } else if s.ident == "VariadicArgs" {
                            variant = ArgVariant::Variadic;
                        } else {
                            variant = ArgVariant::Generic;
                        }
                    }
                }
            }
            _ => {}
        }

        Ok(Self { index, ty, variant })
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum ReturnType {
    Undefined,
    Type(TypeT),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SelfType {
    NoSelf,
    SelfRef,
    SelfMutRef,
}

impl ReturnType {
    pub(crate) fn parse(ret: &syn::ReturnType) -> Result<Self, &'static str> {
        Ok(match ret {
            syn::ReturnType::Default => Self::Undefined,
            syn::ReturnType::Type(_, ty) => Self::Type(Type::parse(ty, true)?.ty),
        })
    }
}

pub fn parse_impl(
    bounds: &Punctuated<TypeParamBound, Token![+]>,
) -> Result<Vec<Path>, &'static str> {
    let mut out = Vec::with_capacity(bounds.len());

    for bound in bounds {
        match bound {
            TypeParamBound::Trait(t) => {
                out.push(t.path.clone());
            }
            TypeParamBound::Verbatim(_) => panic!("Verbatim not supported"),
            _ => {} //ignore, they will just be lifetimes
        }
    }

    Ok(out)
}
