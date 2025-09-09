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
    #[error("Proof of quota verification failed")]
    ProofOfQuotaVerificationFailed,
    #[error("Deserialization failed")]
    DeserializationFailed,
    #[error("Invalid payload length")]
    InvalidPayloadLength,
    #[error("Signature verification failed")]
    SignatureVerificationFailed,
    #[error("No proof of quota left")]
    NoProofOfQuotasLeft,
}
