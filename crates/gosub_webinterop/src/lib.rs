extern crate proc_macro;

use proc_macro::TokenStream;
use std::collections::HashMap;
use std::sync::RwLock;

use lazy_static::lazy_static;
use proc_macro2::{Ident, TokenTree};
use quote::ToTokens;
use syn::spanned::Spanned;
use syn::{FnArg, ItemImpl, ItemStruct};

use crate::function::Function;
use crate::impl_function::impl_js_functions;
use crate::impl_interop_struct::impl_interop_struct;
use crate::property::{FieldProperty, FunctionProperty};
use crate::types::{Arg, ArgVariant, Field, GenericsMatcher, ReturnType, SelfType};
use crate::utils::crate_name;

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
pub fn web_interop(_: TokenStream, item: TokenStream) -> TokenStream {
    let mut fields: Vec<Field> = Vec::new();

    let mut input: ItemStruct = syn::parse_macro_input!(item);

    for field in &mut input.fields {
        if let Some(property) = FieldProperty::parse(&mut field.attrs) {
            let f = Field {
                name: property
                    .rename
                    .unwrap_or(field.ident.as_ref().unwrap().to_string()),
                executor: property.executor,
                ty: field.ty.clone(),
                ident: field.ident.as_ref().unwrap().clone(),
            };

            fields.push(f);
        }
    }

    let extend = impl_interop_struct(input.ident.clone(), &fields);

    let name = input.ident.clone().into_token_stream().to_string();
    STATE.write().unwrap().insert((crate_name(), name), 0);

    let mut out = input.into_token_stream();
    out.extend(extend);

    out.into()
}

#[proc_macro_attribute]
pub fn web_fns(attr: TokenStream, item: TokenStream) -> TokenStream {
    // let item = preprocess_variadic(item); // custom `...` syntax for variadic functions, but it breaks code editors
    let mut input: ItemImpl = {
        let item = item;
        syn::parse_macro_input!(item)
    };

    let mut functions: Vec<Function> = Vec::new();

    for func in &mut input.items {
        if let syn::ImplItem::Fn(method) = func {
            let args = &method.sig.inputs;

            let property = FunctionProperty::parse(&mut method.attrs).unwrap_or_default();

            let name = property.rename.unwrap_or(method.sig.ident.to_string());
            let mut func = Function {
                ident: Ident::new(&name, method.sig.ident.span()),
                name,
                arguments: Vec::with_capacity(args.len()), // we don't know if the first is self, so no args.len() - 1
                self_type: SelfType::NoSelf,
                return_type: ReturnType::parse(&method.sig.output)
                    .expect("failed to parse return type"),
                executor: property.executor,
                generics: GenericsMatcher::get_matchers(property.generics, method),
                func_generics: method.sig.generics.clone(),
                variadic: false,
                needs_ctx: false,
            };

            if let Some(FnArg::Receiver(self_arg)) = args.first() {
                assert!(self_arg.reference.is_some(), "Self must be a reference");

                match self_arg.mutability {
                    Some(_) => func.self_type = SelfType::SelfMutRef,
                    None => func.self_type = SelfType::SelfRef,
                };
            }

            let mut index = 0;
            for arg in args {
                if let FnArg::Typed(arg) = arg {
                    let arg =
                        Arg::parse(&arg.ty, index, &func.generics).expect("failed to parse arg");
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
                        assert!(!(arg.variant != ArgVariant::Context), "Variadic argument must be the last argument111");
                        //get second last
                        assert!(func.arguments.len() > 1, "Variadic argument must be the last argument222");
                        if let Some(arg) = func.arguments.get(func.arguments.len() - 2) {
                            assert!(!(arg.variant != ArgVariant::Variadic), "Variadic argument must be the last argument333");
                        }
                    }
                }
            }

            if func.needs_ctx {
                if let Some(arg) = func.arguments.last() {
                    assert!(!(arg.variant != ArgVariant::Context), "Context argument must be the last argument");
                }
            }

            functions.push(func);
        }
    }

    let name = Ident::new(
        &input.self_ty.to_token_stream().to_string(),
        input.self_ty.span(),
    );

    let options = parse_attrs(attr);

    let extend = impl_js_functions(&functions, &name, &options);

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
