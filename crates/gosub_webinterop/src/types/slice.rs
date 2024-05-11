use crate::types::{Type, TypeT};
use proc_macro2::{Ident, TokenStream};
use quote::quote;

pub fn handle_slice_conv(root_arg: &Type, arg_name: &Ident) -> TokenStream {
    if let TypeT::Array(arg, len) = &root_arg.ty {
        if len.is_none() {
            return TokenStream::new();
        }

        let ty = array_type(arg);

        let conv = handle_slice_conv_recursive(arg, arg_name, true);

        let mutability = root_arg.mutability(true);

        quote! {

            #conv

            let Ok(#mutability #arg_name) = <#ty>::try_from(#arg_name) else {
                cb.error("failed to convert argument");
                return;
            };
        }
    } else {
        TokenStream::new()
    }
}

fn handle_slice_conv_recursive(arg: &Type, arg_name: &Ident, first: bool) -> TokenStream {
    if let TypeT::Array(arg, _len) = &arg.ty {
        let ty = array_type(arg);

        let conv = handle_slice_conv_recursive(arg, arg_name, false);

        if first {
            return quote! {
                let Some(#arg_name): Option<Vec<#ty>> = #arg_name.into_iter().map(|#arg_name| {
                    #conv
                    <#ty>::try_from(#arg_name).ok()
                }).collect::<Option<_>>() else {
                    cb.error("failed to convert argument");
                    return;
                };
            };
        }

        quote! {
            let #arg_name: Vec<#ty> = #arg_name.into_iter().map(|#arg_name| {
                #conv
                <#ty>::try_from(#arg_name).ok()
            }).collect::<Option<_>>()?;
        }
    } else {
        TokenStream::new()
    }
}

fn array_type(arg: &Type) -> TokenStream {
    match &arg.ty {
        TypeT::Type(_) | TypeT::Generic(_) => quote! { _ },
        TypeT::Array(t, len) => {
            let ty = array_type(t);
            let len = if let Some(len) = len {
                quote! {; #len }
            } else {
                TokenStream::new()
            };
            quote! { [#ty #len] }
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
