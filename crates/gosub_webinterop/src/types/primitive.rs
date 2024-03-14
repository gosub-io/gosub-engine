use proc_macro2::{Ident, TokenStream};
use quote::quote;

#[derive(Clone, PartialEq, Debug)]
pub(crate) enum Primitive {
    Number,
    String,
    Boolean,
    UndefinedNull,
    Object,
}

impl Primitive {
    pub(crate) fn get(ty: &str) -> Self {
        let ty = ty.replace('&', "");
        match &*ty {
            "i8" | "u8" | "i16" | "u16" | "i32" | "u32" | "i64" | "u64" | "i128" | "u128"
            | "isize" | "usize" | "f32" | "f64" => Primitive::Number,
            "String" | "&str" => Primitive::String,
            "bool" => Primitive::Boolean,
            "()" => Primitive::UndefinedNull,
            _ => Primitive::Object,
        }
    }

    pub(crate) fn get_check(&self, arg_name: &Ident) -> TokenStream {
        match self {
            Primitive::Number => quote! { #arg_name.is_number() },
            Primitive::String => quote! { #arg_name.is_string() },
            Primitive::Boolean => quote! { #arg_name.is_boolean() },
            Primitive::UndefinedNull => quote! { #arg_name.is_undefined() || #arg_name.is_null() },
            Primitive::Object => quote! { #arg_name.is_object() }, //TODO we need better checks here, (e.g strict check, so fields are matched too)
        }
    }
}
