use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, parse_macro_input};

/// The `SimpleEncoding` macro implements `Encoding`, [`From`] and [`TryFrom`] for tuple structs with exactly one field (e.g.: `struct MyType(T);`).
///
/// # Provides
///
/// - Implements the `Encoding` trait for the struct.
/// - Implements `From<Self>` for `EncodingFormat`, wrapping the struct in the corresponding enum variant.
/// - Implements `TryFrom<EncodingFormat>` for `Self`, extracting the struct from the enum or returning an error if the variant does not match.
///
/// # Limitations
///
/// - Only works for single-field tuple structs.
/// - The struct's name must match the corresponding `EncodingFormat` enum variant.
/// - The single field is treated as the wrapped type.
#[proc_macro_derive(SimpleEncoding)]
pub fn derive_simple_encoding(input: TokenStream) -> TokenStream {
    let input: DeriveInput = parse_macro_input!(input);
    let ty_ident = input.ident;
    let var_ident = ty_ident.clone();
    let enum_path: syn::Path = syn::parse_quote!(EncodingFormat);

    let expanded = quote! {
        impl Encoding for #ty_ident {}

        // impl From<T> for EncodingFormat
        // TODO: `EncodingAdapter` just requires `Into<EncodingFormat> for T`. Should we simplify?
        impl From<#ty_ident> for #enum_path {
            fn from(value: #ty_ident) -> Self {
                Self::#var_ident(value)
            }
        }

        // impl TryFrom<EncodingFormat> for T
        impl ::core::convert::TryFrom<#enum_path> for #ty_ident {
            type Error = crate::encodings::EncodingError;
            fn try_from(encoding: #enum_path) -> Result<Self, Self::Error> {
                if let #enum_path::#var_ident(inner) = encoding {
                    Ok(inner)
                } else {
                    Err(Self::Error::Requires(EncodingKind::#var_ident))
                }
            }
        }
    };

    TokenStream::from(expanded)
}
