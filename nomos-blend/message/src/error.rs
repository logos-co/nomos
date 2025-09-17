use crate::crypto::proofs::{quota, selection};

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Encapsulation count exceeded")]
    EncapsulationCountExceeded,
    #[error("Empty encapsulation inputs")]
    EmptyEncapsulationInputs,
    #[error("Payload too large")]
    PayloadTooLarge,
    #[error("Proof of selection verification failed")]
    ProofOfSelectionVerificationFailed(selection::Error),
    #[error("Proof of quota verification failed")]
    ProofOfQuotaVerificationFailed(quota::Error),
    #[error("Deserialization failed")]
    DeserializationFailed,
    #[error("Invalid payload length")]
    InvalidPayloadLength,
    #[error("Signature verification failed")]
    SignatureVerificationFailed,
    #[error("Node is not a core node")]
    NotCoreNodeReceiver,
    #[error("Node has generated the maximum number of allowed Proof of Quota this session")]
    NoMoreProofOfQuotas,
}
