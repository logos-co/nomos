mod encoding_bytes;
mod encoding_format;
mod errors;
mod kind;

pub use encoding_bytes::Bytes;
pub use encoding_format::{EncodingFormat, EncodingFormatAdapter};
pub use errors::EncodingError;
pub use kind::EncodingKind;

/// Represents an encoding provided by the KMS crate.
///
/// # Consistency
///
/// Due to the `#[derive(SimpleEncodingFormat)]` macro and [`EncodingKind`],
/// variants require the same name as the wrapped type. To use a different name:
/// - Implement [`EncodingFormat`], [`TryFrom`], and [`Into`] traits manually.
/// - Match the variant in [`EncodingKind`]'s [`From<&Encoding>`].
/// - Adjust the [`Display`] implementation in [`EncodingKind`].
pub enum Encoding {
    Bytes(Bytes),
}

impl EncodingFormat for Encoding {}

impl From<bytes::Bytes> for Encoding {
    fn from(value: bytes::Bytes) -> Self {
        Self::Bytes(value.into())
    }
}
