use key_management_system_macros::SimpleEncoding;

use crate::encodings::{Encoding, EncodingFormat, EncodingKind};

type Inner = bytes::Bytes;

/// An encoding of arbitrary bytes.
#[derive(SimpleEncoding)]
pub struct Bytes(Inner);

impl From<Inner> for Bytes {
    fn from(value: Inner) -> Self {
        Self(value)
    }
}
