use std::fmt::Display;

use crate::encodings::Encoding;

/// # Consistency
///
/// Represents an encoding kind provided by the KMS crate.
///
/// # Consistency
///
/// Each variant corresponds directly to a variant in [`Encoding`].
/// The variant names must match exactly with those in [`Encoding`] for
/// consistency, as this is relied upon by the [`Display`] implementation and
/// conversion logic.
///
/// To know more, read [`Encoding`]'s `Consistency` section.
///
/// # Display
///
/// The [`Display`] implementation relies on the naming consistency to print the
/// encoding kind and its corresponding type path (e.g.:
/// `EncodingKind::Bytes(crate::encodings::Bytes)`).
///
/// # Example
///
/// ```
/// use crate::encodings::{Encoding, EncodingKind};
///
/// let encoding = Encoding::Bytes(vec![1, 2, 3]);
/// let encoding_kind = EncodingKind::from(&encoding);
/// assert_eq!(encoding_kind.to_string(), "EncodingKind::Bytes(crate::encodings::Bytes)");
/// ```
#[derive(Debug)]
pub enum EncodingKind {
    Bytes,
}

impl From<&Encoding> for EncodingKind {
    fn from(value: &Encoding) -> Self {
        match value {
            Encoding::Bytes(_) => Self::Bytes,
        }
    }
}

impl Display for EncodingKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "EncodingKind::{self:?}(crate::encodings::{self:?})")
    }
}
