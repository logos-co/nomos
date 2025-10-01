use proc_macro2::{Ident, Span, TokenStream};
use quote::quote;
use syn::{
    Attribute, Data, DeriveInput, Error, Field, FieldMutability, Fields, FieldsUnnamed, Type,
    Variant, Visibility, parse_quote,
    punctuated::{Pair, Punctuated},
    token::{Comma, Paren},
};

#[proc_macro_derive(KmsEnumKey)]
pub fn derive_kms_enum_key(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = syn::parse_macro_input!(input as DeriveInput);
    let Data::Enum(key_enum) = &input.data else {
        return Error::new_spanned(input.ident, "KmsEnumKey can only be derived for enums")
            .to_compile_error()
            .into();
    };

    // If there are no variants, do nothing. This won't implement `SecuredKey` or
    // the encodings, but it will allow the crate to be compiled when no keys
    // are enabled.
    if key_enum.variants.is_empty() {
        return quote! {}.into();
    }

    if let Some(err) = validate_variants(&key_enum.variants) {
        return err.to_compile_error().into();
    }

    let key_enum_variants = &key_enum.variants;
    let encoding_enums = build_encoding_enums(key_enum_variants);
    let key_enum_impl_secured_key =
        build_key_enum_impl_secured_key(&input.ident, key_enum_variants);

    quote! {
        #encoding_enums
        #key_enum_impl_secured_key
    }
    .into()
}

/// Validate each variant must have exactly one unnamed field (the key type).
fn validate_variants(key_enum_variants: &Punctuated<Variant, Comma>) -> Option<Error> {
    for variant in key_enum_variants {
        if !matches!(&variant.fields, Fields::Unnamed(f) if f.unnamed.len() == 1) {
            return Some(Error::new_spanned(
                variant,
                "KmsEnumKey expects each variant to have exactly one unnamed field (the key type)",
            ));
        }
    }
    None
}

fn build_encoding_enums(key_enum_variants: &Punctuated<Variant, Comma>) -> TokenStream {
    let payload_encoding_type_ident = &Ident::new("Payload", Span::call_site());
    let signature_encoding_type_ident = &Ident::new("Signature", Span::call_site());
    let public_key_encoding_type_ident = &Ident::new("PublicKey", Span::call_site());

    let payload_encoding_enum = build_encoding_enum(payload_encoding_type_ident, key_enum_variants);
    let signature_encoding_enum =
        build_encoding_enum(signature_encoding_type_ident, key_enum_variants);
    let public_key_encoding_enum =
        build_encoding_enum(public_key_encoding_type_ident, key_enum_variants);

    quote! {
        #payload_encoding_enum
        #signature_encoding_enum
        #public_key_encoding_enum
    }
}

fn build_encoding_enum(
    encoding_type: &Ident,
    key_enum_variants: &Punctuated<Variant, Comma>,
) -> TokenStream {
    let encoding_enum_ident = Ident::new(&format!("{encoding_type}Encoding"), Span::call_site());
    let encoding_enum_variants = key_enum_variants
        .iter()
        .map(|variant| build_encoding_enum_variant(encoding_type, variant));

    quote! {
        pub enum #encoding_enum_ident {
            #(#encoding_enum_variants),*
        }
    }
}

fn build_encoding_enum_variant(encoding_type: &Ident, key_enum_variant: &Variant) -> Variant {
    let variant_ident = key_enum_variant.ident.clone();
    let variant_attributes_cfg: Vec<Attribute> =
        get_cfg_attributes(&key_enum_variant.attrs).collect();

    let wrapped_key = get_wrapped_key_as_ref(key_enum_variant);
    let wrapped_key_type = &wrapped_key.ty;
    let wrapped_key_encoding_type: Type =
        parse_quote!(<#wrapped_key_type as crate::keys::secured_key::SecuredKey>::#encoding_type);

    let wrapped_type_field = Field {
        attrs: vec![],
        vis: Visibility::Inherited,
        mutability: FieldMutability::None,
        ident: None,
        colon_token: None,
        ty: wrapped_key_encoding_type,
    };
    let unnamed = Punctuated::<Field, _>::from_iter(vec![Pair::End(wrapped_type_field)]);
    let field = Fields::Unnamed(FieldsUnnamed {
        paren_token: Paren::default(),
        unnamed,
    });
    Variant {
        attrs: variant_attributes_cfg,
        ident: variant_ident,
        fields: field,
        discriminant: None,
    }
}

fn build_key_enum_impl_secured_key(
    ident: &Ident,
    key_enum_variants: &Punctuated<Variant, Comma>,
) -> TokenStream {
    let key_enum_impl_secured_key_method_sign =
        build_key_enum_impl_secured_key_method_sign(key_enum_variants);
    let key_enum_impl_secured_key_method_as_public_key =
        build_key_enum_impl_secured_key_method_as_public_key(key_enum_variants);

    quote! {
        impl crate::keys::secured_key::SecuredKey for #ident
        {
            type Payload = PayloadEncoding;
            type Signature = SignatureEncoding;
            type PublicKey = PublicKeyEncoding;
            type Error = crate::keys::errors::KeyError;

            #key_enum_impl_secured_key_method_sign
            #key_enum_impl_secured_key_method_as_public_key
        }
    }
}

fn build_key_enum_impl_secured_key_method_sign(
    key_enum_variants: &Punctuated<Variant, Comma>,
) -> TokenStream {
    let match_block = if key_enum_variants.is_empty() {
        quote! {
            Err(Self::Error::NoKeysEnabled)
        }
    } else {
        let sign_arms_ok = key_enum_variants.iter().map(|variant| {
            let variant_ident = &variant.ident;
            let variant_attributes_cfg: Vec<Attribute> =
                get_cfg_attributes(&variant.attrs).collect();

            quote! {
                #(#variant_attributes_cfg)*
                (Self::#variant_ident(key), Self::Payload::#variant_ident(payload)) => {
                    key.sign(payload).map(Self::Signature::#variant_ident)
                }
            }
        });

        let sign_arm_error = quote! {
            (key, payload) => Err(crate::keys::errors::EncodingError::requires(key, payload).into()),
        };

        quote! {
            match (self, payload) {
                #(#sign_arms_ok)*
                #sign_arm_error
            }
        }
    };

    quote! {
        fn sign(&self, payload: &Self::Payload) -> Result<Self::Signature, Self::Error> {
            #match_block
        }
    }
}

fn build_key_enum_impl_secured_key_method_as_public_key(
    key_enum_variants: &Punctuated<Variant, Comma>,
) -> TokenStream {
    let match_block = if key_enum_variants.is_empty() {
        quote! {
            Err(Self::Error::NoKeysEnabled)
        }
    } else {
        let as_public_key_arms = key_enum_variants.iter().map(|variant| {
            let variant_ident = &variant.ident;
            let variant_attributes_cfg: Vec<Attribute> = get_cfg_attributes(&variant.attrs).collect();
            quote! {
                #(#variant_attributes_cfg)*
                Self::#variant_ident(key) => key.as_public_key().map(Self::PublicKey::#variant_ident),
            }
        });

        quote! {
            match self {
                #(#as_public_key_arms)*
            }
        }
    };

    quote! {
        fn as_public_key(&self) -> Result<Self::PublicKey, Self::Error> {
            #match_block
        }
    }
}

fn get_wrapped_key_as_ref(variant: &Variant) -> &Field {
    match &variant.fields {
        Fields::Unnamed(field) if field.unnamed.len() == 1 => field.unnamed.first().unwrap(),
        _ => unreachable!("Validated in `validate_variants`"),
    }
}

fn is_cfg(attribute: &Attribute) -> bool {
    attribute.path().is_ident("cfg")
}

fn get_cfg_attributes(attributes: &[Attribute]) -> impl Iterator<Item = Attribute> {
    attributes
        .iter()
        .filter(|attribute| is_cfg(attribute))
        .cloned()
}
