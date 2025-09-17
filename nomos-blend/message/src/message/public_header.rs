use serde::{Deserialize, Serialize};

use crate::{
    crypto::{keys::Ed25519PublicKey, proofs::quota::ProofOfQuota, signatures::Signature},
    Error,
};

const LATEST_BLEND_MESSAGE_VERSION: u8 = 1;

// A public header that is revealed to all nodes.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
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

    pub const fn signing_pubkey(&self) -> &Ed25519PublicKey {
        &self.signing_pubkey
    }

    pub const fn signature(&self) -> &Signature {
        &self.signature
    }

    pub const fn into_components(self) -> (u8, Ed25519PublicKey, ProofOfQuota, Signature) {
        (
            self.version,
            self.signing_pubkey,
            self.proof_of_quota,
            self.signature,
        )
    }

    #[cfg(any(test, feature = "unsafe-test-functions"))]
    pub const fn signature_mut(&mut self) -> &mut Signature {
        &mut self.signature
    }
}
