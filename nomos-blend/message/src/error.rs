#[derive(thiserror::Error, Debug, PartialEq, Eq)]
pub enum Error {
    #[error("Invalid message format")]
    InvalidMessageFormat,
    #[error("Max encapsulations exceeded")]
    MaxEncapsulationsExceeded,
    #[error("Empty encapsulation inputs")]
    EmptyEncapsulationInputs,
    #[error("Payload too large")]
    PayloadTooLarge,
    #[error("Proof of selection verification failed")]
    ProofOfSelectionVerificationFailed,
    #[error("Invalid payload length")]
    InvalidPayloadLength,
}
