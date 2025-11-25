use derivative::Derivative;
use nomos_core::sdp::SessionNumber;
use serde::Serialize;
use tracing::debug;

use crate::{
    encap::ProofsVerifier as ProofsVerifierTrait,
    reward::{LOG_TARGET, token::BlendingToken},
};

/// An activity proof for a session, made of the blending token
/// that has the smallest Hamming distance satisfying the activity threshold.
#[derive(Derivative, Serialize)]
#[derivative(Debug)]
pub struct ActivityProof {
    session_number: SessionNumber,
    #[derivative(Debug = "ignore")]
    token: BlendingToken,
}

impl ActivityProof {
    #[must_use]
    pub(crate) const fn new(session_number: SessionNumber, token: BlendingToken) -> Self {
        Self {
            session_number,
            token,
        }
    }

    #[must_use]
    pub const fn session_number(&self) -> SessionNumber {
        self.session_number
    }

    #[must_use]
    pub const fn token(&self) -> &BlendingToken {
        &self.token
    }

    pub fn verify<ProofsVerifier: ProofsVerifierTrait>(
        &self,
        verifier: &ProofsVerifier,
        node_index: u64,
        membership_size: u64,
    ) -> Result<(), ProofsVerifier::Error> {
        self.token.verify(verifier, node_index, membership_size)
    }
}

/// Sensitivity parameter to control the lottery winning conditions.
const ACTIVITY_THRESHOLD_SENSITIVITY_PARAM: u64 = 1;

/// Computes the activity threshold, which is the expected maximum Hamming
/// distance from any blending token in a session to the next session
/// randomness.
pub fn activity_threshold(token_count_bit_len: u64, network_size_bit_len: u64) -> u64 {
    debug!(
        target: LOG_TARGET,
        "Calculating activity threshold: token_count_bit_len={token_count_bit_len}, network_size_repr_bit_len={network_size_bit_len}"
    );

    token_count_bit_len
        .saturating_sub(network_size_bit_len)
        .saturating_sub(ACTIVITY_THRESHOLD_SENSITIVITY_PARAM)
}

impl From<&ActivityProof> for nomos_core::sdp::BlendActivityProof {
    fn from(proof: &ActivityProof) -> Self {
        Self {
            session: proof.session_number,
            proof_of_quota: (&proof.token.proof_of_quota).into(),
            signing_key: proof.token.signing_key.into(),
            proof_of_selection: (&proof.token.proof_of_selection).into(),
        }
    }
}

impl TryFrom<&nomos_core::sdp::BlendActivityProof> for ActivityProof {
    type Error = Box<dyn std::error::Error>;

    fn try_from(proof: &nomos_core::sdp::BlendActivityProof) -> Result<Self, Self::Error> {
        Ok(Self::new(
            proof.session,
            BlendingToken::new(
                (&proof.proof_of_quota).try_into()?,
                proof.signing_key.try_into()?,
                (&proof.proof_of_selection).try_into()?,
            ),
        ))
    }
}
