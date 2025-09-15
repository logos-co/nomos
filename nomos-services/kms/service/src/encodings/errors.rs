use thiserror::Error;

use crate::encodings::EncodingKind;

/// Errors that can occur when encoding or decoding.
#[derive(Error, Debug)]
pub enum EncodingError {
    #[error("Required encoding: {0}")]
    Requires(EncodingKind),
}
