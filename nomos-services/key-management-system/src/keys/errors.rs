use thiserror::Error;

use crate::encodings::EncodingError;

#[derive(Error, Debug)]
pub enum KeyError {
    #[error(transparent)]
    Encoding(EncodingError),
}

impl From<EncodingError> for KeyError {
    fn from(value: EncodingError) -> Self {
        Self::Encoding(value)
    }
}
