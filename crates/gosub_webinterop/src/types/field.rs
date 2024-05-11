use proc_macro2::TokenStream;
use quote::quote;

use crate::types::executor::Executor;

#[derive(Clone, Debug)]
pub struct Field {
    pub(crate) name: String,
    pub(crate) ident: syn::Ident,
    #[allow(unused)]
    pub(crate) executor: Executor,
    //needed later
    pub(crate) ty: syn::Type,
}

impl Field {
    pub(crate) fn getters_setters(fields: &[Self]) -> TokenStream {
        let mut gs = TokenStream::new();

        for field in fields {
            gs.extend(field.getter_setter());
        }

        gs
    }

    fn getter_setter(&self) -> TokenStream {
        let getters = self.getter();
        let setters = self.setter();

        let field_name = &self.name;

        quote! {
            {
                #getters
                #setters

                obj.set_property_accessor(#field_name, getter, setter)?;
            }
        }
    }

    fn getter(&self) -> TokenStream {
        let ident = &self.ident;
        quote! {
            let getter = {
                let s = Rc::clone(&s);
                Box::new(move |cb: &mut RT::GetterCB| {
                    let ctx = cb.context();
                    let value = s.borrow().#ident;
                    let value = match value.to_js_value(ctx.clone()) {
                        Ok(value) => value,
                        Err(e) => {
                            cb.error(e);
                            return;
                        }
                    };
                    cb.ret(value);
                })
            };
        }
    }

    fn setter(&self) -> TokenStream {
        let ident = &self.ident;
        let field_type = &self.ty;
        quote! {
            let setter = {
                let s = Rc::clone(&s);
                Box::new(move |cb: &mut RT::SetterCB| {
                    let value = cb.value();
                    let value: #field_type = match value.to_rust_value() {
                        Ok(value) => value,
                        Err(e) => {
                            cb.error(e);
                            return;
                        }
                    };
                    s.borrow_mut().#ident = value;
                })
            };
        }
    }
}
