use derivative::Derivative;
use nomos_core::sdp::SessionNumber;
use serde::Serialize;
use tracing::debug;

use crate::reward::{LOG_TARGET, token::BlendingToken};

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

    #[cfg(test)]
    pub(crate) const fn token(&self) -> &BlendingToken {
        &self.token
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
