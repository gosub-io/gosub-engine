use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};
use syn::{GenericParam, Path};

use crate::types::executor::Executor;
use crate::types::{Arg, ArgVariant, Generics, Primitive, ReturnType, SelfType};

#[derive(Clone, Debug)]
pub struct Function {
    pub(crate) name: String,
    pub(crate) ident: Ident,
    pub(crate) arguments: Vec<Arg>,
    pub(crate) self_type: SelfType,
    #[allow(unused)]
    pub(crate) return_type: ReturnType,
    //needed later
    pub(crate) executor: Executor,
    pub(crate) generics: Vec<Generics>,
    pub(crate) func_generics: syn::Generics,
    pub(crate) variadic: bool,
    pub(crate) needs_ctx: bool,
}

impl Function {
    pub(crate) fn implement(&self, name: &Ident) -> TokenStream {
        assert!(self.executor.is_js());

        let ident = &self.ident;
        let func_name = &self.name;
        let mut num_args = self.arguments.len();

        let var_func = if self.variadic {
            quote! { FunctionVariadic }
        } else {
            quote! { Function }
        };
        let set_method = if self.variadic {
            quote! { set_method_variadic }
        } else {
            quote! { set_method }
        };

        let args_call = self.args_and_call(name);

        if self.needs_ctx {
            num_args -= 1;
        }

        let len_check = if self.variadic {
            num_args -= 1;

            quote! {
                if cb.len() <= #num_args  {
                cb.error("wrong number of arguments");
                return;
                }
            }
        } else {
            quote! {
                if cb.len() != #num_args  {
                cb.error("wrong number of arguments");
                return;
                }
            }
        };

        let clone = if self.self_type == SelfType::NoSelf {
            TokenStream::new()
        } else {
            quote! {  let s = Rc::clone(&s); }
        };

        let args = if self.arguments.is_empty() {
            TokenStream::new()
        } else {
            quote! { let args = cb.args(); }
        };

        quote! {
        let #ident = {
            #clone
            RT::#var_func::new(ctx.clone(), move |cb| {
                #len_check

                let ctx = cb.context();

                #args

                #args_call
            })?
        };

        obj.#set_method(#func_name, &#ident)?;
        }
    }

    fn args_and_call(&self, name: &Ident) -> TokenStream {
        if !self.generics.is_empty() {
            return self.generic_call(name);
        }

        let prepare_args = self.prepare_args();

        let call = self.call(name);
        quote! {
        #prepare_args

        #call
        }
    }

    fn prepare_args(&self) -> TokenStream {
        if self.arguments.is_empty() {
            return TokenStream::new();
        }

        let mut prepared_args = Vec::with_capacity(self.arguments.len());

        for arg in &self.arguments {
            prepared_args.push(arg.prepare());
        }

        quote! {
            #(#prepared_args)*
        }
    }

    fn generic_call(&self, name: &Ident) -> TokenStream {
        let non_generic = self.prepare_args();
        let call = self.call(name);

        let mut generic_args = Vec::new();

        for arg in &self.arguments {
            if arg.variant == ArgVariant::Generic {
                generic_args.push(arg);
            }
        }

        let generic = self.generic(&mut generic_args, call);

        quote! {
            #non_generic

            #generic
        }
    }

    fn match_generics(&self, arg: &Arg) -> Vec<(Path, Primitive)> {
        let matches: Vec<_> = self
            .generics
            .iter()
            .filter_map(|matcher| {
                if matcher
                    .matcher
                    .is_match(&arg.ty.ty.generics().unwrap(), arg.index)
                {
                    Some(matcher.types.clone())
                } else {
                    None
                }
            })
            .collect();

        assert_eq!(
            matches.len(),
            1,
            "Multiple or no matches found for generic type: {} matches, expected 1",
            matches.len()
        );

        matches.first().unwrap().clone()
    }

    fn generic(&self, args: &mut Vec<&Arg>, prev: TokenStream) -> TokenStream {
        let Some(arg) = args.pop() else {
            return prev;
        };
        let arg_name = format_ident!("arg{}", arg.index);

        let mut out = TokenStream::new();

        let js_types = self.match_generics(arg);

        for js_ty in js_types {
            let check = js_ty.1.get_check(&arg_name);
            let ty = js_ty.0;

            out.extend(quote! {
                if #check {
                    let Ok(#arg_name): Result<#ty> = #arg_name.to_rust_value() else {
                            cb.error("failed to convert argument");
                            return;
                        };

                    #prev
                } else
            });
        }

        out.extend(quote! {
            {
                cb.error("failed to convert argument");
                return;
            }
        });

        self.generic(args, out)
    }

    pub(crate) fn call_args(&self) -> TokenStream {
        if self.arguments.is_empty() {
            return TokenStream::new();
        }

        let mut call_args = Vec::with_capacity(self.arguments.len());
        for (index, arg) in self.arguments.iter().enumerate() {
            call_args.push(arg.call(index));
        }

        quote! {
        #(#call_args),*
        }
    }
    fn call(&self, name: &Ident) -> TokenStream {
        let ident = &self.ident;
        let call_args = self.call_args();

        let func = {
            match self.self_type {
                SelfType::NoSelf => quote! { #name::#ident },
                SelfType::SelfRef => quote! { s.borrow().#ident },
                SelfType::SelfMutRef => quote! { s.borrow_mut().#ident },
            }
        };
        let func_generics = self.get_generics();

        quote! {
            let ret = match #func #func_generics(#call_args).to_js_value(ctx.clone()) {
                Ok(ret) => ret,
                Err(e) => {
                    cb.error(e);
                    return;
                }
            };
            cb.ret(ret);
        }
    }

    fn get_generics(&self) -> TokenStream {
        if !self
            .func_generics
            .params
            .iter()
            .any(|gen| matches!(gen, GenericParam::Type(_)))
        {
            return TokenStream::new();
        }

        let mut generics = Vec::new();

        for gen in &self.func_generics.params {
            if let GenericParam::Type(p) = &gen {
                if p.ident == "VariadicArgs" {
                    generics.push(quote! { RT::VariadicArgs });
                    continue;
                }
                if p.ident == "RT" || p.ident == "Runtime" {
                    generics.push(quote! { RT });
                    continue;
                }

                generics.push(quote! { _ });
            }
        }

        quote! { ::<#(#generics),*> }
    }
}
