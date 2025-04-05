use proc_macro2::{Span, TokenStream};
use quote::ToTokens;

#[derive(Debug, Clone, Copy)]
enum Format {
    Json,
    Ron,
    Toml,
    Txt,
    Yaml,
}

impl Format {
    fn path(self) -> TokenStream {
        match self {
            Format::Json => quote::quote!(::assets_manager::asset::load_json),
            Format::Ron => quote::quote!(::assets_manager::asset::load_ron),
            Format::Toml => quote::quote!(::assets_manager::asset::load_toml),
            Format::Txt => quote::quote!(::assets_manager::asset::load_text),
            Format::Yaml => quote::quote!(::assets_manager::asset::load_yaml),
        }
    }

    fn extensions(self) -> TokenStream {
        match self {
            Format::Json => quote::quote!(&["json"]),
            Format::Ron => quote::quote!(&["ron"]),
            Format::Toml => quote::quote!(&["toml"]),
            Format::Txt => quote::quote!(&["txt"]),
            Format::Yaml => quote::quote!(&["yaml", "yml"]),
        }
    }
}

pub fn run(input: syn::DeriveInput) -> syn::Result<TokenStream> {
    let format = get_format(&input.attrs)?;
    check_fields(&input.data)?;

    let loader = format.path();
    let ext = format.extensions();

    let asset = input.ident;

    let (impl_gen, ty_gen, where_gen) = input.generics.split_for_impl();
    let mut where_gen = where_gen.cloned().unwrap_or_else(|| syn::WhereClause {
        where_token: Default::default(),
        predicates: Default::default(),
    });
    add_clauses(&mut where_gen, format);

    Ok(quote::quote! {
        impl #impl_gen ::assets_manager::FileAsset for #asset #ty_gen #where_gen {
            const EXTENSIONS: &'static [&'static str] = #ext;
            fn from_bytes(bytes: ::std::borrow::Cow<[u8]>) -> Result<Self, ::assets_manager::BoxedError> {
                #loader(&bytes)
            }
        }
    })
}

fn add_clauses(generics: &mut syn::WhereClause, format: Format) {
    generics
        .predicates
        .push(syn::parse_quote!(Self: ::std::marker::Send + ::std::marker::Sync + 'static));

    let trait_clause = match format {
        Format::Json | Format::Ron | Format::Toml | Format::Yaml => {
            syn::parse_quote!(Self: for<'de> ::serde::Deserialize<'de>)
        }
        Format::Txt => syn::parse_quote!(Self: ::std::str::FromStr),
    };
    generics.predicates.push(trait_clause);
}

fn is_format_attribute(meta: &syn::Meta) -> bool {
    meta.path().get_ident().is_some_and(|i| i == "asset_format")
}

fn get_format(attrs: &[syn::Attribute]) -> syn::Result<Format> {
    let mut formats = None;

    for attr in attrs {
        if !is_format_attribute(&attr.meta) {
            continue;
        }

        if formats.is_some() {
            return Err(syn::Error::new_spanned(
                attr,
                "found multiple asset formats",
            ));
        }

        let meta = attr.meta.require_name_value()?;
        let name = syn::parse2::<syn::LitStr>(meta.value.to_token_stream())?;

        let format = match name.value().as_str() {
            "json" => Format::Json,
            "ron" => Format::Ron,
            "toml" => Format::Toml,
            "txt" => Format::Txt,
            "yml" | "yaml" => Format::Yaml,
            s => {
                return Err(syn::Error::new(
                    name.span(),
                    format_args!("unsupported format: {s:?}"),
                ));
            }
        };

        formats = Some(format);
    }

    formats.ok_or_else(|| syn::Error::new(Span::call_site(), "missing asset format"))
}

fn check_fields(data: &syn::Data) -> syn::Result<()> {
    let check_attrs = |attrs: &[syn::Attribute]| {
        for attr in attrs {
            if is_format_attribute(&attr.meta) {
                return Err(syn::Error::new_spanned(attr, "unexpected attribute"));
            }
        }
        Ok(())
    };

    match data {
        syn::Data::Struct(s) => {
            for field in &s.fields {
                check_attrs(&field.attrs)?;
            }
        }
        syn::Data::Enum(e) => {
            for variant in &e.variants {
                check_attrs(&variant.attrs)?;

                for field in &variant.fields {
                    check_attrs(&field.attrs)?;
                }
            }
        }
        syn::Data::Union(u) => {
            for field in &u.fields.named {
                check_attrs(&field.attrs)?;
            }
        }
    }

    Ok(())
}
