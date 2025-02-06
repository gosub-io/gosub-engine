use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};

use crate::types::Field;

pub fn impl_interop_struct(name: Ident, fields: &[Field], js_name: TokenStream) -> TokenStream {
    let marker_struct = format_ident!("{}JSMethodsMarker", name);
    let marker_trait = format_ident!("{}JSMethods", name);

    let getters_setters = Field::getters_setters(fields);

    quote! {
        impl JSInterop for #name {
            fn implement<RT: WebRuntime>(s: Rc<RefCell<Self>>, mut ctx: RT::Context) -> Result<()> {
                let mut obj = RT::Object::new(&ctx)?;

                ctx.set_on_global_object(stringify!(#js_name), obj.clone().into())?; //#name

                #getters_setters

                (&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&&#marker_struct)
                    .implement::<RT>(&mut obj, s, ctx)?;

                Ok(())
            }
        }

        struct #marker_struct;
        trait #marker_trait {
            fn implement<RT: WebRuntime>(&self, obj: &mut RT::Object, s: Rc<RefCell<#name>>, ctx: RT::Context) -> Result<()>;
        }

        impl #marker_trait for #marker_struct {
            #[inline(always)]
            fn implement<RT: WebRuntime>(&self, _: &mut RT::Object, _: Rc<RefCell<#name>>, _: RT::Context) -> Result<()> {
                Ok(())
            }
        }
    }
}
