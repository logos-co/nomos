mod encoding;
mod encoding_bytes;
mod encoding_fr;
mod errors;
mod kind;

pub use encoding::{Encoding, EncodingAdapter};
pub use encoding_bytes::Bytes;
pub use encoding_fr::Fr;
pub use errors::EncodingError;
pub use kind::EncodingKind;

/// Represents an encoding provided by the KMS crate.
///
/// # Consistency
///
/// Due to the `#[derive(SimpleEncoding)]` macro and [`EncodingKind`], variants
/// require the same name as the wrapped type. To use a different name:
/// - Implement [`Encoding`], [`TryFrom`], and [`Into`] traits manually.
/// - Match the variant in [`EncodingKind`]'s [`From<&EncodingFormat>`].
/// - Adjust the [`Display`] implementation in [`EncodingKind`].
pub enum EncodingFormat {
    Bytes(Bytes),
    Fr(Fr),
}

impl Encoding for EncodingFormat {}

impl From<bytes::Bytes> for EncodingFormat {
    fn from(value: bytes::Bytes) -> Self {
        Self::Bytes(value.into())
    }
}

impl From<groth16::Fr> for EncodingFormat {
    fn from(value: groth16::Fr) -> Self {
        Self::Fr(value.into())
    }
}
