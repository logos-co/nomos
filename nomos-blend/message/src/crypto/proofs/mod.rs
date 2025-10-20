use core::mem::swap;

use nomos_core::crypto::{ZkHash, ZkHasher};
use thiserror::Error;

use crate::{
    crypto::{
        keys::Ed25519PublicKey,
        proofs::{
            quota::{
                ProofOfQuota,
                inputs::prove::{
                    PublicInputs,
                    public::{CoreInputs, LeaderInputs},
                },
            },
            selection::{ProofOfSelection, inputs::VerifyInputs},
        },
    },
    encap::ProofsVerifier,
};

pub mod quota;
pub mod selection;

/// The inputs required to verify a Proof of Quota, without the signing key,
/// which is retrieved from the public header of the message layer being
/// verified.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(test, derive(Default))]
pub struct PoQVerificationInputsMinusSigningKey {
    pub session: u64,
    pub core: CoreInputs,
    pub leader: LeaderInputs,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Invalid Proof of Quota: {0}.")]
    ProofOfQuota(#[from] quota::Error),
    #[error("Invalid Proof of Selection: {0}.")]
    ProofOfSelection(selection::Error),
}

/// Verifier that actually verifies the validity of Blend-related proofs.
#[derive(Clone)]
pub struct RealProofsVerifier {
    current_inputs: PoQVerificationInputsMinusSigningKey,
    previous_epoch_inputs: Option<LeaderInputs>,
}

impl ProofsVerifier for RealProofsVerifier {
    type Error = Error;

    fn new(public_inputs: PoQVerificationInputsMinusSigningKey) -> Self {
        Self {
            current_inputs: public_inputs,
            previous_epoch_inputs: None,
        }
    }

    fn start_epoch_transition(&mut self, new_pol_inputs: LeaderInputs) {
        let old_epoch_inputs = {
            let mut new_pol_inputs = new_pol_inputs;
            swap(&mut self.current_inputs.leader, &mut new_pol_inputs);
            new_pol_inputs
        };
        self.previous_epoch_inputs = Some(old_epoch_inputs);
    }

    fn complete_epoch_transition(&mut self) {
        self.previous_epoch_inputs = None;
    }

    fn verify_proof_of_quota(
        &self,
        proof: ProofOfQuota,
        signing_key: &Ed25519PublicKey,
    ) -> Result<ZkHash, Self::Error> {
        let PoQVerificationInputsMinusSigningKey {
            core,
            leader,
            session,
        } = self.current_inputs;

        // Try with current input, and if it fails, try with the previous one, if any
        // (i.e., within the epoch transition period).
        proof
            .verify(&PublicInputs {
                core,
                leader,
                session,
                signing_key: *signing_key,
            })
            .or_else(|_| {
                let Some(previous_epoch_inputs) = self.previous_epoch_inputs else {
                    return Err(Error::ProofOfQuota(quota::Error::InvalidProof));
                };
                proof
                    .verify(&PublicInputs {
                        core,
                        leader: previous_epoch_inputs,
                        session,
                        signing_key: *signing_key,
                    })
                    .map_err(Error::ProofOfQuota)
            })
    }

    fn verify_proof_of_selection(
        &self,
        proof: ProofOfSelection,
        inputs: &VerifyInputs,
    ) -> Result<(), Self::Error> {
        proof.verify(inputs).map_err(Error::ProofOfSelection)
    }
}

trait ZkHashExt {
    fn hash(&self) -> ZkHash;
}

impl<T> ZkHashExt for T
where
    T: AsRef<[ZkHash]>,
{
    fn hash(&self) -> ZkHash {
        let mut hasher = ZkHasher::new();
        hasher.update(self.as_ref());
        hasher.finalize()
    }
}
