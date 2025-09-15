use thiserror::Error;

use crate::encodings::EncodingError;

#[derive(Error, Debug)]
pub enum KeyError {
    #[error(transparent)]
    Encoding(EncodingError),
    #[error("An error happened: {0}")]
    Any(String), // TODO: Temporary debugging while implementing.
}

impl From<EncodingError> for KeyError {
    fn from(value: EncodingError) -> Self {
        Self::Encoding(value)
    }
}
