use thiserror::Error;

use crate::encodings::EncodingKind;

/// Errors that can occur when encoding or decoding.
#[derive(Error, Debug)]
pub enum EncodingError {
    #[expect(dead_code, reason = "Will be used when adding the ZK key.")]
    #[error("Required encoding: {0}")]
    Requires(EncodingKind),
}
