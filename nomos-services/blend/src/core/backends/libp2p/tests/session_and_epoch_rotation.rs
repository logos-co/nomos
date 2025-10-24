use core::convert::Infallible;

use libp2p::PeerId;
use nomos_blend_message::{
    crypto::{
        keys::Ed25519PublicKey,
        proofs::{
            PoQVerificationInputsMinusSigningKey,
            quota::{ProofOfQuota, inputs::prove::public::LeaderInputs},
            selection::{ProofOfSelection, inputs::VerifyInputs},
        },
    },
    encap::ProofsVerifier,
};
use nomos_core::crypto::ZkHash;
use test_log::test;

use crate::core::backends::{EpochInfo, SessionInfo};

enum Action {
    NewSession(SessionInfo<PeerId>),
    OldSessionExpired,
    NewEpoch(EpochInfo),
    OldEpochExpired,
}

impl From<SessionInfo<PeerId>> for Action {
    fn from(value: SessionInfo<PeerId>) -> Self {
        Self::NewSession(value)
    }
}

impl From<EpochInfo> for Action {
    fn from(value: EpochInfo) -> Self {
        Self::NewEpoch(value)
    }
}

struct HistoryTrackingProofsVerifier(PoQVerificationInputsMinusSigningKey, Vec<Action>);

impl ProofsVerifier for HistoryTrackingProofsVerifier {
    type Error = Infallible;

    fn new(public_inputs: PoQVerificationInputsMinusSigningKey) -> Self {
        Self(public_inputs, vec![])
    }

    fn complete_epoch_transition(&mut self) {
        self.1.push(Action::OldEpochExpired);
    }

    fn start_epoch_transition(&mut self, new_pol_inputs: LeaderInputs) {
        self.1.push(new_pol_inputs.into());
    }

    fn verify_proof_of_quota(
        &self,
        _proof: ProofOfQuota,
        _signing_key: &Ed25519PublicKey,
    ) -> Result<ZkHash, Self::Error> {
        panic!("Should never be called in tests");
    }

    fn verify_proof_of_selection(
        &self,
        _proof: ProofOfSelection,
        _inputs: &VerifyInputs,
    ) -> Result<(), Self::Error> {
        panic!("Should never be called in tests");
    }
}

#[test(tokio::test)]
async fn rotate_session() {
    // TODO: Add these tests
}

#[test(tokio::test)]
async fn complete_session_transition() {
    // TODO: Add these tests
}

#[test(tokio::test)]
async fn rotate_epoch() {
    // TODO: Add these tests
}

#[test(tokio::test)]
async fn complete_epoch_transition() {
    // TODO: Add these tests
}
