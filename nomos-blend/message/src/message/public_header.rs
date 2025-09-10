use groth16::Fr;
use serde::{Deserialize, Serialize};

use crate::{
    crypto::{
        keys::Ed25519PublicKey,
        proofs::quota::{inputs::prove::PublicInputs, ProofOfQuota},
        signatures::Signature,
    },
    Error,
};

const LATEST_BLEND_MESSAGE_VERSION: u8 = 1;

// A public header that is revealed to all nodes.
#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct PublicHeader {
    version: u8,
    signing_pubkey: Ed25519PublicKey,
    proof_of_quota: ProofOfQuota,
    signature: Signature,
}

impl PublicHeader {
    pub const fn new(
        signing_pubkey: Ed25519PublicKey,
        proof_of_quota: &ProofOfQuota,
        signature: Signature,
    ) -> Self {
        Self {
            proof_of_quota: *proof_of_quota,
            signature,
            signing_pubkey,
            version: LATEST_BLEND_MESSAGE_VERSION,
        }
    }

    pub fn verify_signature(&self, body: &[u8]) -> Result<(), Error> {
        if self.signing_pubkey.verify_signature(body, &self.signature) {
            Ok(())
        } else {
            Err(Error::SignatureVerificationFailed)
        }
    }

    pub fn verify_proof_of_quota(&self, verification_inputs: &PublicInputs) -> Result<Fr, Error> {
        self.proof_of_quota
            .verify(*verification_inputs)
            .map_err(|_| Error::ProofOfQuotaVerificationFailed)
    }

    pub const fn signing_pubkey(&self) -> &Ed25519PublicKey {
        &self.signing_pubkey
    }

    pub const fn signature(&self) -> &Signature {
        &self.signature
    }

    #[cfg(any(test, feature = "unsafe-test-functions"))]
    pub const fn signature_mut(&mut self) -> &mut Signature {
        &mut self.signature
    }
}
