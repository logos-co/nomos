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
