#[derive(thiserror::Error, Debug, PartialEq, Eq)]
pub enum Error {
    #[error("Encapsulation count exceeded")]
    EncapsulationCountExceeded,
    #[error("Empty encapsulation inputs")]
    EmptyEncapsulationInputs,
    #[error("Payload too large")]
    PayloadTooLarge,
    #[error("Proof of selection verification failed")]
    ProofOfSelectionVerificationFailed,
    #[error("Deserialization failed")]
    DeserializationFailed,
    #[error("Invalid payload length")]
    InvalidPayloadLength,
}
