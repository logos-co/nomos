mod encoding_bytes;
mod errors;
mod kind;

pub use encoding_bytes::Bytes;
pub use errors::EncodingError;
pub use kind::EncodingKind;

/// Entity that gathers all encodings provided by the KMS crate.
pub enum Encoding {
    Bytes(Bytes),
}

impl From<Bytes> for Encoding {
    fn from(value: Bytes) -> Self {
        Self::Bytes(value)
    }
}

impl From<bytes::Bytes> for Encoding {
    fn from(value: bytes::Bytes) -> Self {
        Self::Bytes(value.into())
    }
}
