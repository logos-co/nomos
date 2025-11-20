use serde::{Deserialize, Serialize};

use crate::{
    Error, MessageIdentifier,
    crypto::{
        keys::Ed25519PublicKey,
        proofs::quota::{self, ProofOfQuota, VerifiedProofOfQuota},
        signatures::Signature,
    },
    encap::ProofsVerifier,
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

    pub fn verify_proof_of_quota<Verifier>(&self, verifier: &Verifier) -> Result<(), Error>
    where
        Verifier: ProofsVerifier,
    {
        verifier
            .verify_proof_of_quota(self.proof_of_quota, &self.signing_pubkey)
            .map_err(|_| Error::ProofOfQuotaVerificationFailed(quota::Error::InvalidProof))?;
        Ok(())
    }

    pub const fn signing_pubkey(&self) -> &Ed25519PublicKey {
        &self.signing_pubkey
    }

    pub const fn proof_of_quota(&self) -> &ProofOfQuota {
        &self.proof_of_quota
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

    #[cfg(test)]
    pub const fn proof_of_quota_mut(&mut self) -> &mut ProofOfQuota {
        &mut self.proof_of_quota
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct VerifiedPublicHeader {
    version: u8,
    proof_of_quota: VerifiedProofOfQuota,
    signing_pubkey: Ed25519PublicKey,
    signature: Signature,
}

impl From<VerifiedPublicHeader> for PublicHeader {
    fn from(
        VerifiedPublicHeader {
            proof_of_quota,
            signature,
            signing_pubkey,
            ..
        }: VerifiedPublicHeader,
    ) -> Self {
        Self::new(signing_pubkey, &proof_of_quota.into(), signature)
    }
}

impl VerifiedPublicHeader {
    pub fn new(
        proof_of_quota: VerifiedProofOfQuota,
        signing_pubkey: Ed25519PublicKey,
        signature: Signature,
    ) -> Self {
        let (version, signing_pubkey, _, signature) =
            PublicHeader::new(signing_pubkey, proof_of_quota.as_ref(), signature).into_components();
        Self {
            version,
            proof_of_quota,
            signing_pubkey,
            signature,
        }
    }

    pub fn from_header_unchecked(header: &PublicHeader) -> Self {
        Self {
            version: header.version,
            signing_pubkey: header.signing_pubkey,
            proof_of_quota: VerifiedProofOfQuota::from_proof_of_quota_unchecked(
                header.proof_of_quota,
            ),
            signature: header.signature,
        }
    }

    #[must_use]
    pub const fn proof_of_quota(&self) -> &VerifiedProofOfQuota {
        &self.proof_of_quota
    }

    pub const fn id(&self) -> MessageIdentifier {
        self.signing_pubkey
    }

    #[cfg(any(feature = "unsafe-test-functions", test))]
    pub const fn signature_mut(&mut self) -> &mut Signature {
        &mut self.signature
    }

    #[must_use]
    pub const fn into_components(self) -> (u8, Ed25519PublicKey, VerifiedProofOfQuota, Signature) {
        (
            self.version,
            self.signing_pubkey,
            self.proof_of_quota,
            self.signature,
        )
    }
}
