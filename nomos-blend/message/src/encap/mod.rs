use nomos_core::crypto::ZkHash;

use crate::crypto::{
    keys::Ed25519PublicKey,
    proofs::{
        PoQVerificationInputsMinusSigningKey,
        quota::{ProofOfQuota, inputs::prove::public::LeaderInputs},
        selection::{ProofOfSelection, inputs::VerifyInputs},
    },
};

pub mod decapsulated;
pub mod encapsulated;
pub mod validated;

#[cfg(test)]
mod tests;

/// A session-bound `PoQ` verifier.
pub trait ProofsVerifier {
    type Error;

    /// Create a new proof verifier with the public inputs corresponding to the
    /// current Blend session and cryptarchia epoch.
    fn new(public_inputs: PoQVerificationInputsMinusSigningKey) -> Self;

    /// Start a new epoch while still maintaining the old one around for
    /// messages that are propagated around the bound between two epochs.
    fn start_epoch_transition(&mut self, new_pol_inputs: LeaderInputs);
    /// Complete the transition period and discard any messages generated in the
    /// previous epoch.
    fn complete_epoch_transition(&mut self, old_epoch_nonce: ZkHash);

    /// Proof of Quota verification logic.
    fn verify_proof_of_quota(
        &self,
        proof: ProofOfQuota,
        signing_key: &Ed25519PublicKey,
    ) -> Result<ZkHash, Self::Error>;

    /// Proof of Selection verification logic.
    fn verify_proof_of_selection(
        &self,
        proof: ProofOfSelection,
        inputs: &VerifyInputs,
    ) -> Result<(), Self::Error>;
}
