extern crate proc_macro;

use parking_lot::RwLock;
use proc_macro::TokenStream;
use std::collections::HashMap;

use crate::function::Function;
use crate::impl_function::impl_js_functions;
use crate::impl_interop_struct::impl_interop_struct;
use crate::property::{FieldProperty, FunctionProperty};
use crate::types::{Arg, ArgVariant, Field, GenericsMatcher, ReturnType, SelfType};
use crate::utils::crate_name;
use lazy_static::lazy_static;
use proc_macro2::{Ident, TokenTree};
use quote::ToTokens;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{parse_macro_input, FnArg, ItemImpl, ItemStruct, MetaNameValue, Token};

mod function;
mod impl_function;
mod impl_interop_struct;
mod property;
mod types;
mod utils;

lazy_static! {
    static ref STATE: RwLock<HashMap<(String, String), u8>> = RwLock::new(HashMap::new());
}

#[proc_macro_attribute]
pub fn web_interop(args: TokenStream, item: TokenStream) -> TokenStream {
    let mut fields: Vec<Field> = Vec::new();

    let mut input: ItemStruct = syn::parse_macro_input!(item);

    for field in &mut input.fields {
        let property = match FieldProperty::parse(&mut field.attrs) {
            Ok(property) => property,
            Err(e) => return e.to_compile_error().into(),
        };
        if let Some(property) = property {
            let Some(ident) = field.ident.as_ref() else {
                return syn::Error::new_spanned(&*field, "#[property] is only supported on named struct fields")
                    .to_compile_error()
                    .into();
            };
            let f = Field {
                name: property.rename.unwrap_or(ident.to_string()),
                executor: property.executor,
                ty: field.ty.clone(),
                ident: ident.clone(),
            };

            fields.push(f);
        }
    }

    let items: Punctuated<MetaNameValue, Token![,]> =
        parse_macro_input!(args with Punctuated::<MetaNameValue, Token![,]>::parse_terminated);

    let mut js_name = input.ident.to_token_stream();

    for item in items {
        if item.path.is_ident("js_name") {
            js_name = item.value.to_token_stream();
        }
    }

    let extend = impl_interop_struct(input.ident.clone(), &fields, js_name);

    let name = input.ident.clone().into_token_stream().to_string();
    STATE.write().insert((crate_name(), name), 0);

    let mut out = input.into_token_stream();
    out.extend(extend);

    out.into()
}

#[proc_macro_attribute]
pub fn web_fns(attr: TokenStream, item: TokenStream) -> TokenStream {
    // let item = preprocess_variadic(item); // custom `...` syntax for variadic functions, but it breaks code editors
    let mut input: ItemImpl = {
        let item = item.clone();
        syn::parse_macro_input!(item)
    };

    let mut functions: Vec<Function> = Vec::new();

    for func in &mut input.items {
        if let syn::ImplItem::Fn(method) = func {
            let args = &method.sig.inputs;

            let property = match FunctionProperty::parse(&mut method.attrs) {
                Ok(property) => property.unwrap_or_default(),
                Err(e) => return e.to_compile_error().into(),
            };

            let name = property.rename.unwrap_or(method.sig.ident.to_string());
            let return_type = match ReturnType::parse(&method.sig.output) {
                Ok(return_type) => return_type,
                Err(e) => return syn::Error::new_spanned(&method.sig.output, e).to_compile_error().into(),
            };
            let generics = match GenericsMatcher::get_matchers(property.generics, method) {
                Ok(generics) => generics,
                Err(e) => return e.to_compile_error().into(),
            };
            let mut func = Function {
                ident: Ident::new(&name, method.sig.ident.span()),
                name,
                arguments: Vec::with_capacity(args.len()), // we don't know if the first is self, so no args.len() - 1
                self_type: SelfType::NoSelf,
                return_type,
                executor: property.executor,
                generics,
                func_generics: method.sig.generics.clone(),
                variadic: false,
                needs_ctx: false,
            };

            if let Some(FnArg::Receiver(self_arg)) = args.first() {
                if self_arg.reference.is_none() {
                    return syn::Error::new_spanned(self_arg, "self must be a reference")
                        .to_compile_error()
                        .into();
                }

                match self_arg.mutability {
                    Some(_) => func.self_type = SelfType::SelfMutRef,
                    None => func.self_type = SelfType::SelfRef,
                }
            }

            let mut index = 0;
            for arg in args {
                if let FnArg::Typed(arg) = arg {
                    let arg = match Arg::parse(&arg.ty, index, &func.generics) {
                        Ok(arg) => arg,
                        Err(e) => return syn::Error::new_spanned(&arg.ty, e).to_compile_error().into(),
                    };
                    if arg.variant == ArgVariant::Variadic {
                        func.variadic = true;
                    } else if arg.variant == ArgVariant::Context {
                        func.needs_ctx = true;
                    }
                    func.arguments.push(arg);
                    index += 1;
                }
            }

            if func.variadic {
                if let Some(arg) = func.arguments.last() {
                    if arg.variant != ArgVariant::Variadic {
                        if arg.variant != ArgVariant::Context {
                            return syn::Error::new_spanned(&method.sig.ident, "variadic argument must be the last argument")
                                .to_compile_error()
                                .into();
                        }
                        //get second last
                        if func.arguments.len() <= 1 {
                            return syn::Error::new_spanned(&method.sig.ident, "variadic argument must be the last argument")
                                .to_compile_error()
                                .into();
                        }
                        if let Some(arg) = func.arguments.get(func.arguments.len() - 2) {
                            if arg.variant != ArgVariant::Variadic {
                                return syn::Error::new_spanned(&method.sig.ident, "variadic argument must be the last argument")
                                    .to_compile_error()
                                    .into();
                            }
                        }
                    }
                }
            }

            if func.needs_ctx {
                if let Some(arg) = func.arguments.last() {
                    if arg.variant != ArgVariant::Context {
                        return syn::Error::new_spanned(&method.sig.ident, "context argument must be the last argument")
                            .to_compile_error()
                            .into();
                    }
                }
            }

            functions.push(func);
        }
    }

    let name = Ident::new(&input.self_ty.to_token_stream().to_string(), input.self_ty.span());

    let options = parse_attrs(attr);

    let extend = match impl_js_functions(&functions, &name, &options) {
        Ok(extend) => extend,
        Err(e) => return e.to_compile_error().into(),
    };

    let mut out = input.into_token_stream();
    out.extend(extend);

    out.into()
}

struct Options {
    refs: Option<u8>,
    marker_struct: Option<Ident>,
    marker_trait: Option<Ident>,
}

fn parse_attrs(attrs: TokenStream) -> Options {
    let attrs: proc_macro2::TokenStream = attrs.into();
    let mut options = Options {
        refs: None,
        marker_struct: None,
        marker_trait: None,
    };

    for item in attrs {
        match item {
            TokenTree::Ident(i) => {
                if options.marker_struct.is_none() {
                    options.marker_struct = Some(i);
                } else if options.marker_trait.is_none() {
                    options.marker_trait = Some(i);
                }
            }

            TokenTree::Literal(l) => {
                let num = l.to_string();
                let Ok(num) = num.parse::<u8>() else { continue };

                options.refs = Some(num);
            }

            _ => {}
        }
    }

    options
}
