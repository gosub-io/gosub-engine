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

    let marker_struct = options
        .marker_struct
        .as_ref()
        .map_or_else(|| format_ident!("{}JSMethodsMarker", name), |marker_struct| {
            marker_struct.clone()
        });

    let marker_trait = options
        .marker_trait
        .as_ref()
        .map_or_else(|| format_ident!("{}JSMethods", name), |marker_trait| {
            marker_trait.clone()
        });

    let refs = get_refs(name.to_string(), options.refs);

    quote! {
        impl #marker_trait for #refs #marker_struct {
            #[inline(always)]
            fn implement<RT: WebRuntime>(&self, obj: &mut RT::Object, s: Rc<RefCell<#name>>, ctx: RT::Context) -> Result<()> {
                #(#impls)*

                (*self).implement::<RT>(obj, s, ctx)
            }
        }
    }
}

#[allow(clippy::significant_drop_tightening)]
fn get_refs(name: String, num_refs: Option<u8>) -> TokenStream {
    let num_refs = num_refs.unwrap_or_else(|| {
        let mut state = STATE.write().unwrap();
        let num_refs = state.get_mut(&(crate_name(), name)).unwrap();
        *num_refs += 1;
        *num_refs
    });
    let mut refs = TokenStream::new();

    for _ in 0..num_refs {
        refs.extend(quote! { & });
    }

    refs
}
