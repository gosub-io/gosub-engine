//! This file is currently unused.
//! It generates a custom syntax for variadic functions, but it breaks code editors.


use proc_macro::TokenStream;
use proc_macro2::{Delimiter, Spacing, TokenStream as TokenStream2, TokenTree};
use proc_macro2::Punct;
use quote::quote;

#[allow(unused)]
pub(crate) fn preprocess_variadic(item: TokenStream) -> TokenStream {
    let mut item: TokenStream2 = item.into();

    let mut iter = item.clone().into_iter();

    let mut out = TokenStream2::new();
    
    while let Some(tt) = iter.next() {
        match &tt {
            TokenTree::Ident(i) => {
                if i == "impl" {
                    out.extend(std::iter::once(tt));
                    'inner: for tt in iter.by_ref() {
                        if let TokenTree::Group(g) = tt {
                            
                            let mut group = TokenStream2::new();
                            expand_impl(g.stream(), &mut group);
                            
                            push_group(&mut out, group, g.delimiter());
                            break 'inner;
                        } else {
                            out.extend(std::iter::once(tt));
                        }
                    }
                    
                    
                } else {
                    out.extend(std::iter::once(tt));
                }


            }
            _ => {
                out.extend(std::iter::once(tt));
            }
        }
    }

    out.into()
}

fn push_group(out: &mut TokenStream2, stream: TokenStream2, delimiter: Delimiter) {
    out.extend(std::iter::once(TokenTree::Group(proc_macro2::Group::new(delimiter, stream))));
    
}

fn expand_impl(stream: TokenStream2, out: &mut TokenStream2) {
    let mut iter = stream.into_iter();

    while let Some(tt) = iter.next() {
        match &tt {
            TokenTree::Ident(i) => {
                if i == "fn" {
                    out.extend(std::iter::once(tt));
                    for tt in iter.by_ref() {
                        if let TokenTree::Group(g) = tt {
                            let mut group = TokenStream2::new();
                            expand_args(g.stream(), &mut group);
                            
                            push_group(out, group, g.delimiter());
                            break;
                        } else {
                            out.extend(std::iter::once(tt));
                        }
                    }
                }
            }
            _ => {
                out.extend(std::iter::once(tt));
            }
        }
    }
}

fn expand_args(stream: TokenStream2, out: &mut TokenStream2) {
    let mut found = false;

    let mut dot_count = 0;

    let mut iter = stream.into_iter();

    let mut dots = TokenStream2::new();

    while let Some(tt) = iter.next() {
        match &tt {
            TokenTree::Punct(p) => {
                if p.as_char() == '.' {
                    dot_count += 1;
                    dots.extend(std::iter::once(tt));
                } else {
                    dot_count = 0;
                    out.extend(dots.clone().into_iter());
                    out.extend(std::iter::once(tt));
                }

                if dot_count == 3 {
                    let name = iter.next().expect("expected name after ...");

                    let arg = quote!{
                        #name: &impl VariadicArgs
                    };

                    out.extend(arg.into_iter());

                    assert!(iter.next().is_none(), "variadic args must be the last argument");
                }
            }
            _ => {
                dot_count = 0;
                out.extend(dots.clone().into_iter());
                out.extend(std::iter::once(tt));
            }
        }
    }
}