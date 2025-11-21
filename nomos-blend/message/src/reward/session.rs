use std::ops::Add as _;

use nomos_core::{blend::SessionRandomness, crypto::ZkHash, sdp::SessionNumber};
use nomos_utils::math::{F64Ge1, NonNegativeF64};
use tracing::debug;

use crate::reward::LOG_TARGET;

/// Session-specific information to compute an activity proof.
pub struct SessionInfo {
    pub(crate) session_number: SessionNumber,
    pub(crate) session_randomness: SessionRandomness,
    pub(crate) token_count_byte_len: u64,
    pub(crate) activity_threshold: u64,
}

impl SessionInfo {
    pub fn new(
        session_number: SessionNumber,
        pol_epoch_nonce: &ZkHash,
        num_core_nodes: u64,
        core_quota: u64,
    ) -> Result<Self, Error> {
        let total_core_quota = core_quota
            .checked_mul(num_core_nodes)
            .ok_or(Error::TotalCoreQuotaTooLarge(u64::MAX))?;

        let network_size_bit_len = F64Ge1::try_from(
            num_core_nodes
                .checked_add(1)
                .ok_or(Error::NetworkSizeTooLarge(num_core_nodes))?,
        )
        .map_err(|()| Error::NetworkSizeTooLarge(num_core_nodes))?
        .log2()
        .ceil() as u64;

        let token_count_bit_len = token_count_bit_len(total_core_quota)?;
        let activity_threshold = activity_threshold(token_count_bit_len, network_size_bit_len);

        let session_randomness = SessionRandomness::new(session_number, pol_epoch_nonce);

        Ok(Self {
            session_number,
            session_randomness,
            token_count_byte_len: token_count_bit_len.div_ceil(8),
            activity_threshold,
        })
    }

    #[must_use]
    pub const fn session_randomness(&self) -> SessionRandomness {
        self.session_randomness
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the total core quota({0}) is too large to compute the Hamming distance")]
    TotalCoreQuotaTooLarge(u64),
    #[error("the network size({0}) is too large to compute the activity threshold")]
    NetworkSizeTooLarge(u64),
}

/// The number of bits that can represent the maximum number of blending
/// tokens generated during a single session.
fn token_count_bit_len(total_core_quota: u64) -> Result<u64, Error> {
    let total_core_quota: NonNegativeF64 = total_core_quota
        .try_into()
        .map_err(|()| Error::TotalCoreQuotaTooLarge(total_core_quota))?;
    Ok(F64Ge1::try_from(total_core_quota.add(1.0))
        .expect("must be >= 1.0")
        .log2()
        .ceil() as u64)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_activity_threshold() {
        let network_size_bit_len = 7;
        let token_count_bit_len = 10;
        let threshold = activity_threshold(token_count_bit_len, network_size_bit_len);
        // 10 - 7 - 1
        assert_eq!(threshold, 2);

        let network_size_bit_len = 0;
        let token_count_bit_len = 10;
        let threshold = activity_threshold(token_count_bit_len, network_size_bit_len);
        // 10 - 0 - 1
        assert_eq!(threshold, 9);

        let network_size_bit_len = 7;
        let token_count_bit_len = 0;
        let threshold = activity_threshold(token_count_bit_len, network_size_bit_len);
        // 0 - 7 - 1 (by saturated_sub)
        assert_eq!(threshold, 0);
    }

    #[test]
    fn test_token_count_bit_len() {
        let total_core_quota = 10;
        // ceil(log2(10 + 1))
        assert_eq!(token_count_bit_len(total_core_quota).unwrap(), 4);

        let total_core_quota = 0;
        // ceil(log2(0 + 1))
        assert_eq!(token_count_bit_len(total_core_quota).unwrap(), 0);
    }
}
