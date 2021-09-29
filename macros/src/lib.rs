//! This crate provides the `embed!` macro for [`assets_manager`](https://docs.rs/assets_manager)

use proc_macro::TokenStream;

mod embedded;

#[proc_macro]
pub fn embed(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as embedded::Input);
    input.expand_dir().unwrap_or_else(to_compile_errors).into()
}

fn to_compile_errors(errors: Vec<syn::Error>) -> proc_macro2::TokenStream {
    let errors = errors.iter().map(|e| e.to_compile_error());

    quote::quote! { #(#errors)* }
}
