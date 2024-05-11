use proc_macro2::Ident;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use crate::function::Function;
use crate::utils::crate_name;
use crate::{Options, STATE};

pub fn impl_js_functions(functions: &[Function], name: &Ident, options: &Options) -> TokenStream {
    let mut impls = Vec::new();
    for function in functions {
        if !function.executor.is_js() {
            continue;
        }

        impls.push(function.implement(name));
    }

    let marker_struct = if let Some(marker_struct) = options.marker_struct.as_ref() {
        marker_struct.clone()
    } else {
        format_ident!("{}JSMethodsMarker", name)
    };

    let marker_trait = if let Some(marker_trait) = options.marker_trait.as_ref() {
        marker_trait.clone()
    } else {
        format_ident!("{}JSMethods", name)
    };

    let refs = get_refs(name.to_string(), options.refs);

    quote! {
        impl #marker_trait for #refs #marker_struct {
            #[inline(always)]
            fn implement<RT: JSRuntime>(&self, obj: &mut RT::Object, s: Rc<RefCell<#name>>, ctx: RT::Context) -> Result<()> {
                #(#impls)*

                (*self).implement::<RT>(obj, s, ctx)
            }
        }
    }
}

fn get_refs(name: String, num_refs: Option<u8>) -> TokenStream {
    let num_refs = num_refs.unwrap_or_else(|| {
        let mut state = STATE.write().unwrap();
        let num_refs = state
            .get_mut(&(crate_name(), name))
            .expect("Struct does not have the #[web_interop] attribute");
        *num_refs += 1;
        *num_refs
    });
    let mut refs = TokenStream::new();

    for _ in 0..num_refs {
        refs.extend(quote! { & });
    }

    refs
}
