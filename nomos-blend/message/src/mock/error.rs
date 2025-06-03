use crate::MessageUnwrapError;

#[derive(thiserror::Error, Debug, PartialEq, Eq)]
pub enum Error {
    #[error("Invalid blend message format")]
    InvalidBlendMessage,
    #[error("Payload is too large")]
    PayloadTooLarge,
    #[error("Invalid number of layers")]
    InvalidNumberOfLayers,
    #[error("Invalid public key")]
    InvalidPublicKey,
}

impl From<Error> for MessageUnwrapError<Error> {
    fn from(e: Error) -> Self {
        Self::Other(e)
    }
}
