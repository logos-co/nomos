use std::fmt::Display;

use crate::encodings::{Encoding, EncodingFormat};

/// # Consistency
///


/// Represents an encoding kind provided by the KMS crate.
///
/// # Consistency
/// 
/// Each variant corresponds directly to a variant in [`EncodingFormat`].
/// The variant names must match exactly with those in [`EncodingFormat`] for
/// consistency, as this is relied upon by the [`Display`] implementation and
/// conversion logic. 
/// 
/// To know more, read [`EncodingFormat`]'s `Consistency` section.
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
/// use crate::encodings::{EncodingFormat, kind::EncodingKind};
///
/// let format = EncodingFormat::Bytes(vec![1, 2, 3]);
/// let kind = EncodingKind::from(&format);
/// assert_eq!(kind.to_string(), "EncodingKind::Bytes(crate::encodings::Bytes)");
/// ```
#[derive(Debug)]
pub enum EncodingKind {
    Bytes,
    Fr,
}

impl From<&EncodingFormat> for EncodingKind {
    fn from(value: &EncodingFormat) -> Self {
        match value {
            EncodingFormat::Bytes(_) => Self::Bytes,
            EncodingFormat::Fr(_) => Self::Fr,
        }
    }
}

impl Display for EncodingKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "EncodingKind::{:?}(crate::encodings::{:?})", self, self)
    }
}
