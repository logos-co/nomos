pub mod mock;
pub mod sphinx;

pub trait BlendMessage {
    type PublicKey;
    type PrivateKey;
    type Error: std::error::Error;

    fn build(payload: &[u8], public_keys: &[Self::PublicKey]) -> Result<Vec<u8>, Self::Error>;
    /// Unwrap the message one layer.
    ///
    /// This function returns the unwrapped message and a boolean indicating
    /// whether the message was fully unwrapped. (False if the message still
    /// has layers to be unwrapped, true otherwise)
    ///
    /// If the input message was already fully unwrapped, or if its format is
    /// invalid, this function must return an error.
    fn unwrap(
        message: &[u8],
        private_key: &Self::PrivateKey,
    ) -> Result<(Vec<u8>, bool), MessageUnwrapError<Self::Error>>;
}

#[derive(thiserror::Error, Debug)]
pub enum MessageUnwrapError<MessageError> {
    #[error("Unwrapping the message is not allowed for this node")]
    NotAllowed,
    #[error(transparent)]
    Other(MessageError),
}
