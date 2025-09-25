use std::any::type_name;

use thiserror::Error;

/// Errors that can occur when encoding or decoding.
#[derive(Error, Debug)]
pub enum EncodingError {
    #[error("Required encoding: {0}")]
    Requires(String),
}

impl EncodingError {
    /// Creates a new `EncodingError::Requires` error.
    /// It contains the path to the required type.
    #[expect(dead_code, reason = "Will be used when integrating KMS.")]
    pub fn requires<T>() -> Self {
        Self::Requires(type_name::<T>().to_owned())
    }
}
