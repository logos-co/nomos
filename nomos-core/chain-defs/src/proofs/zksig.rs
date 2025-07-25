pub use nomos_proof_statements::zksig::ZkSignaturePublic;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DummyZkSignature {
    public_inputs: ZkSignaturePublic,
}

impl DummyZkSignature {
    #[must_use]
    pub const fn prove(public_inputs: ZkSignaturePublic) -> Self {
        Self { public_inputs }
    }
}

pub trait ZkSignatureProof {
    /// Verify the proof against the public inputs.
    fn verify(&self, public_inputs: &ZkSignaturePublic) -> bool;
}

impl ZkSignatureProof for DummyZkSignature {
    fn verify(&self, public_inputs: &ZkSignaturePublic) -> bool {
        public_inputs == &self.public_inputs
    }
}
