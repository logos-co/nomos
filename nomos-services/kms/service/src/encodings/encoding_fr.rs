use key_management_system_macros::SimpleEncoding;

use crate::encodings::{Encoding, EncodingFormat, EncodingKind};

type Inner = groth16::Fr;

/// An encoding of a [`groth16::Fr`].
#[derive(SimpleEncoding)]
pub struct Fr(Inner);

impl From<Inner> for Fr {
    fn from(value: Inner) -> Self {
        Self(value)
    }
}
