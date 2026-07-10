use quote::{format_ident, ToTokens};
use syn::{Attribute, LitStr, Meta};

use crate::types::executor::Executor;
use crate::types::{GenericProperty, Primitive};

pub(crate) struct FieldProperty {
    pub(crate) rename: Option<String>,
    pub(crate) executor: Executor,
}

pub(crate) struct FunctionProperty {
    pub(crate) rename: Option<String>,
    pub(crate) executor: Executor,
    pub(crate) generics: Vec<GenericProperty>,
}

impl Default for FieldProperty {
    fn default() -> Self {
        Self {
            rename: None,
            executor: Executor::Both,
        }
    }
}

impl Default for FunctionProperty {
    fn default() -> Self {
        Self {
            rename: None,
            executor: Executor::Both,
            generics: Vec::new(),
        }
    }
}

impl FieldProperty {
    pub(crate) fn parse(attrs: &mut Vec<Attribute>) -> syn::Result<Option<FieldProperty>> {
        let mut remove_attrs = None;
        let mut property = None;

        for (index, attr) in attrs.iter().enumerate() {
            if attr.path().is_ident("property") {
                let mut prop = FieldProperty {
                    rename: None,
                    executor: Executor::Both,
                };

                //rename = "____", js => rename to name, and it is a js only property
                //rename = "____", wasm => rename to name, and it is a wasm only property
                //rename = "____" => rename to name, and it is a property for both, js and wasm
                //js => name is the same, and it is a js only property
                //wasm => name is the same, and it is a wasm only property
                //<nothing> => name is the same, and it is a property for both, js and wasm

                match &attr.meta {
                    Meta::Path(_) => {}
                    Meta::List(_) => {
                        attr.parse_nested_meta(|meta| {
                            match &meta.path {
                                path if path.is_ident("rename") => {
                                    let lit: LitStr = meta.value()?.parse()?;

                                    prop.rename = Some(lit.value());
                                }
                                path if path.is_ident("js") => {
                                    if prop.executor != Executor::Both {
                                        return Err(syn::Error::new_spanned(
                                            &meta.path,
                                            "executor cannot be specified twice",
                                        ));
                                    }
                                    prop.executor = Executor::JS;
                                }
                                path if path.is_ident("wasm") => {
                                    if prop.executor != Executor::Both {
                                        return Err(syn::Error::new_spanned(
                                            &meta.path,
                                            "executor cannot be specified twice",
                                        ));
                                    }
                                    prop.executor = Executor::WASM;
                                }
                                path if path.is_ident("none") => {
                                    if prop.executor != Executor::Both {
                                        return Err(syn::Error::new_spanned(
                                            &meta.path,
                                            "executor cannot be specified twice",
                                        ));
                                    }
                                    prop.executor = Executor::None;
                                }
                                _ => Err(syn::Error::new_spanned(attr, "Unknown attribute in property attribute"))?,
                            }

                            Ok(())
                        })?;
                    }
                    Meta::NameValue(_) => {
                        return Err(syn::Error::new_spanned(
                            attr,
                            "unexpected name-value in property attribute",
                        ));
                    }
                }

                property = Some(prop);
                remove_attrs = Some(index);
            }
        }

        if let Some(index) = remove_attrs {
            attrs.remove(index);
        }

        Ok(property)
    }
}

impl FunctionProperty {
    pub(crate) fn parse(attrs: &mut Vec<Attribute>) -> syn::Result<Option<FunctionProperty>> {
        let mut remove_attrs = Vec::new();
        let mut property = None;

        for (index, attr) in attrs.iter().enumerate() {
            if attr.path().is_ident("property") {
                let mut prop = FunctionProperty::default();

                match &attr.meta {
                    Meta::Path(_) => {}
                    Meta::List(_) => {
                        attr.parse_nested_meta(|meta| {
                            match &meta.path {
                                path if path.is_ident("rename") => {
                                    let lit: LitStr = meta.value()?.parse()?;

                                    prop.rename = Some(lit.value());
                                }
                                path if path.is_ident("js") => {
                                    if prop.executor != Executor::Both {
                                        return Err(syn::Error::new_spanned(
                                            &meta.path,
                                            "executor cannot be specified twice",
                                        ));
                                    }
                                    prop.executor = Executor::JS;
                                }
                                path if path.is_ident("wasm") => {
                                    if prop.executor != Executor::Both {
                                        return Err(syn::Error::new_spanned(
                                            &meta.path,
                                            "executor cannot be specified twice",
                                        ));
                                    }
                                    prop.executor = Executor::WASM;
                                }
                                path if path.is_ident("none") => {
                                    if prop.executor != Executor::Both {
                                        return Err(syn::Error::new_spanned(
                                            &meta.path,
                                            "executor cannot be specified twice",
                                        ));
                                    }
                                    prop.executor = Executor::None;
                                }

                                path => Err(syn::Error::new_spanned(
                                    attr,
                                    format_ident!(
                                        "Unknown attribute in property attribute {}",
                                        path.to_token_stream().to_string()
                                    ),
                                ))?,
                            }

                            Ok(())
                        })?;
                    }
                    Meta::NameValue(_) => {
                        return Err(syn::Error::new_spanned(
                            attr,
                            "unexpected name-value in property attribute",
                        ));
                    }
                }

                property = Some(prop);
                remove_attrs.push(index);
            }

            if attr.path().is_ident("generic") {
                if property.is_none() {
                    property = Some(FunctionProperty::default());
                }

                if matches!(attr.meta, Meta::List(_)) {
                    let mut name_found = false;
                    let mut param = None;
                    let mut types = Vec::new();
                    attr.parse_nested_meta(|meta| {
                        if name_found {
                            let prim = Primitive::get(&meta.path.to_token_stream().to_string());
                            if types.iter().any(|(_, p)| p == &prim) {
                                return Err(syn::Error::new_spanned(
                                    &meta.path,
                                    format!("cannot have multiple {prim:?}s in generic attribute"),
                                ));
                            }
                            types.push((meta.path, prim));
                        } else {
                            param = Some(meta.path);
                            name_found = true;
                        }
                        Ok(())
                    })?;

                    let param =
                        param.ok_or_else(|| syn::Error::new_spanned(attr, "expected param in generic attribute"))?;

                    if let Some(property) = property.as_mut() {
                        property.generics.push(GenericProperty { param, types });
                    }
                } else {
                    return Err(syn::Error::new_spanned(
                        attr,
                        "unexpected name-value in generic attribute",
                    ));
                }

                remove_attrs.push(index);
            }
        }

        for index in remove_attrs {
            attrs.remove(index);
        }

        Ok(property)
    }
}
