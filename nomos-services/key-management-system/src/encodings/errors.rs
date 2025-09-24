use std::fmt::{Debug, Display};

use thiserror::Error;

/// Errors that can occur when encoding or decoding.
#[derive(Error, Debug)]
pub enum EncodingError {
    #[error("Required encoding: {0}")]
    Requires(String),
}

impl EncodingError {
    #[expect(dead_code, reason = "Will be used when adding the ZK key.")]
    pub fn requires<T: Display>(encoding: T) -> Self {
        Self::Requires(encoding.to_string())
    }
}
