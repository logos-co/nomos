use std::{collections::HashSet, ops::Deref};

use blake2::{
    Blake2bVar,
    digest::{Update as _, VariableOutput as _},
};
use nomos_core::{codec::SerializeOp as _, sdp::SessionNumber};
use nomos_utils::math::{F64Ge1, NonNegativeF64};
use serde::Serialize;
use tracing::{debug, error};

use crate::crypto::proofs::{quota::ProofOfQuota, selection::ProofOfSelection};

const LOG_TARGET: &str = "blend::message::reward";

/// Blending tokens being collected for the current session, and tokens retained
/// from the previous session that will later be used to produce an activity
/// proof.
pub struct BlendingTokens {
    current_session_tokens: BlendingTokensForSession,
    current_session_randomness: SessionRandomness,
    previous_session_tokens: Option<BlendingTokensForSession>,
}

impl BlendingTokens {
    #[must_use]
    pub fn new(current_session_info: &SessionInfo) -> Self {
        Self {
            current_session_tokens: BlendingTokensForSession::new(
                current_session_info.session_number,
                current_session_info.token_count_byte_len,
                current_session_info.activity_threshold,
            ),
            current_session_randomness: current_session_info.session_randomness,
            previous_session_tokens: None,
        }
    }

    /// Collects a blending token from the current session.
    pub fn insert(&mut self, token: BlendingToken) {
        self.current_session_tokens.insert(token);
    }

    /// Computes an activity proof for the previous session, if any.
    /// It discards the previous session tokens after computing the proof.
    ///
    /// It returns `None` if there was no blending token collected during
    /// the previous session, or if there is no blending token satisfying the
    /// activity threshold.
    pub fn activity_proof(&mut self) -> Option<ActivityProof> {
        self.previous_session_tokens
            .take()?
            .activity_proof(self.current_session_randomness)
    }

    /// Switches to the new session while retaining the current session tokens,
    /// so that new tokens for the session can continue to be collected during
    /// the session transition period.
    ///
    /// If there was a previous session that has not yet been consumed for
    /// an activity proof, the activity proof is computed and returned.
    pub fn rotate_session(&mut self, new_session_info: &SessionInfo) -> Option<ActivityProof> {
        let activity_proof = self.activity_proof();

        self.previous_session_tokens = Some(std::mem::replace(
            &mut self.current_session_tokens,
            BlendingTokensForSession::new(
                new_session_info.session_number,
                new_session_info.token_count_byte_len,
                new_session_info.activity_threshold,
            ),
        ));
        self.current_session_randomness = new_session_info.session_randomness;

        activity_proof
    }
}

/// Arguments required to build [`BlendingTokens`].
pub struct SessionInfo {
    session_number: SessionNumber,
    session_randomness: SessionRandomness,
    token_count_byte_len: u64,
    activity_threshold: u64,
}

impl SessionInfo {
    pub fn new(
        session_number: SessionNumber,
        session_randomness: SessionRandomness,
        num_core_nodes: u64,
        total_core_quota: u64,
        message_frequency_per_round: NonNegativeF64,
    ) -> Result<Self, Error> {
        let network_size_repr_bit_len = F64Ge1::try_from(
            num_core_nodes
                .checked_add(1)
                .ok_or(Error::NetworkSizeTooLarge(num_core_nodes))?,
        )
        .map_err(|()| Error::NetworkSizeTooLarge(num_core_nodes))?
        .log2()
        .ceil() as u64;

        let token_count_bit_len =
            token_count_bit_len(total_core_quota, message_frequency_per_round)?;

        let activity_threshold = activity_threshold(network_size_repr_bit_len, token_count_bit_len);

        Ok(Self {
            session_number,
            session_randomness,
            token_count_byte_len: token_count_bit_len.div_ceil(8),
            activity_threshold,
        })
    }
}

/// Holds blending tokens and session-specific parameters for a single session.
struct BlendingTokensForSession {
    session_number: SessionNumber,
    token_count_bit_len: u64,
    activity_threshold: u64,
    tokens: HashSet<BlendingToken>,
}

impl BlendingTokensForSession {
    fn new(
        session_number: SessionNumber,
        token_count_bit_len: u64,
        activity_threshold: u64,
    ) -> Self {
        Self {
            session_number,
            token_count_bit_len,
            activity_threshold,
            tokens: HashSet::new(),
        }
    }

    fn insert(&mut self, token: BlendingToken) {
        self.tokens.insert(token);
    }

    /// Computes an activity proof for this session by consuming tokens.
    ///
    /// It returns `None` if there was no blending token satisfying the
    /// activity threshold calculated.
    fn activity_proof(self, next_session_randomness: SessionRandomness) -> Option<ActivityProof> {
        // Find the blending token with the smallest Hamming distance.
        let maybe_token = self
            .tokens
            .into_iter()
            .map(|token| {
                let distance =
                    token.hamming_distance(self.token_count_bit_len, next_session_randomness);
                (token, distance)
            })
            .min_by_key(|(_, distance)| *distance);

        // Check if the smallest distance satisfies the activity threshold.
        if let Some((token, distance)) = maybe_token
            && distance <= self.activity_threshold
        {
            Some(ActivityProof {
                session_number: self.session_number,
                token,
            })
        } else {
            None
        }
    }
}

/// An activity proof for a session, made of the blending token
/// that has the smallest Hamming distance satisfying the activity threshold.
#[expect(dead_code, reason = "Used once integrated with Blend service")]
pub struct ActivityProof {
    session_number: SessionNumber,
    token: BlendingToken,
}

/// A blending token consisting of a proof of quota and a proof of selection.
#[derive(Clone, PartialEq, Eq, Hash, Serialize)]
pub struct BlendingToken {
    proof_of_quota: ProofOfQuota,
    proof_of_selection: ProofOfSelection,
}

impl BlendingToken {
    /// Computes the Hamming distance between this blending token and the next
    /// session randomness.
    fn hamming_distance(
        &self,
        token_count_byte_len: u64,
        next_session_randomness: SessionRandomness,
    ) -> u64 {
        let token = self
            .to_bytes()
            .expect("BlendingToken should be serializable");
        let token_hash = hash(&token, token_count_byte_len as usize);
        let session_randomness_hash = hash(&next_session_randomness, token_count_byte_len as usize);

        hamming_distance(&token_hash, &session_randomness_hash)
    }
}

/// The number of bits that can represent the maximum number of blending
/// tokens generated during a single session.
fn token_count_bit_len(
    total_core_quota: u64,
    message_frequency_per_round: NonNegativeF64,
) -> Result<u64, Error> {
    let total_core_quota: NonNegativeF64 = total_core_quota
        .try_into()
        .map_err(|()| Error::TotalCoreQuotaTooLarge(total_core_quota))?;
    Ok(
        F64Ge1::try_from(total_core_quota.mul_add(message_frequency_per_round.get(), 1.0))
            .expect("must be >= 1.0")
            .log2()
            .ceil() as u64,
    )
}

/// Compute blake-2b hash of `input`, producing `output_size` bytes.
///
/// If `output_size` is greater than the maximum supported size, it will be
/// reduced to that maximum.
/// If `output_size` is zero, an empty vector will be returned.
fn hash(input: &[u8], output_size: usize) -> Vec<u8> {
    let output_size = output_size.min(Blake2bVar::MAX_OUTPUT_SIZE);
    let mut hasher = Blake2bVar::new(output_size).expect("output size should be valid");
    hasher.update(input);
    let mut output = vec![0u8; output_size];
    hasher.finalize_variable(&mut output).unwrap();
    output
}

/// Computes the Hamming distance between two byte slices.
/// (i.e. the number of differing bits)
///
/// If the slices have different lengths, the extra bytes in the longer slice
/// are silently ignored.
fn hamming_distance(a: &[u8], b: &[u8]) -> u64 {
    a.iter()
        .zip(b)
        .map(|(x, y)| u64::from((x ^ y).count_ones()))
        .sum()
}

/// Sensitivity parameter to control the lottery winning conditions.
const ACTIVITY_THRESHOLD_SENSITIVITY_PARAM: u64 = 1;

/// Computes the activity threshold, which is the expected maximum Hamming
/// distance from any blending token in a session to the next session
/// randomness.
fn activity_threshold(network_size_repr_bit_len: u64, token_repr_bit_len: u64) -> u64 {
    debug!(
        target: LOG_TARGET,
        "Calculating activity threshold: network_size_repr_bit_len={network_size_repr_bit_len}, token_repr_bit_len={token_repr_bit_len}"
    );

    token_repr_bit_len
        .saturating_sub(network_size_repr_bit_len)
        .saturating_sub(ACTIVITY_THRESHOLD_SENSITIVITY_PARAM)
}

/// Deterministic unbiased randomness for a session.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct SessionRandomness([u8; 64]);

impl Deref for SessionRandomness {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the total core quota({0}) is too large to compute the Hamming distance")]
    TotalCoreQuotaTooLarge(u64),
    #[error("the network size({0}) is too large to compute the activity threshold")]
    NetworkSizeTooLarge(u64),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::proofs::{quota::PROOF_OF_QUOTA_SIZE, selection::PROOF_OF_SELECTION_SIZE};

    #[test]
    fn test_hamming_distance() {
        assert_eq!(
            hamming_distance(&[0b1010_1010, 0b1100_1100], &[0b1010_1010, 0b1100_1100]),
            0
        );
        assert_eq!(
            hamming_distance(&[0b1010_1010, 0b1100_1100], &[0b0101_0101, 0b0011_0011]),
            16
        );
        assert_eq!(
            hamming_distance(&[0b1111_1111, 0b1111_1111], &[0b0000_0000]),
            8
        );
        assert_eq!(hamming_distance(&[], &[]), 0);
    }

    #[test]
    fn test_hash() {
        let input = b"test data";

        // Check if the output length matches the requested size.
        let output = hash(input, 3);
        assert_eq!(output.len(), 3);

        // Check if `hash` is deterministic.
        assert_eq!(output, hash(input, 3));

        // An empty output if the request size is zero.
        assert!(hash(input, 0).is_empty());

        // Output shouldn't be longer than the maximum size.
        let output = hash(input, Blake2bVar::MAX_OUTPUT_SIZE.checked_add(1).unwrap());
        assert_eq!(output.len(), Blake2bVar::MAX_OUTPUT_SIZE);
    }

    #[test]
    fn test_activity_threshold() {
        let network_size_bit_len = 7;
        let token_repr_bit_len = 10;
        let threshold = activity_threshold(network_size_bit_len, token_repr_bit_len);
        // 10 - 7 - 1
        assert_eq!(threshold, 2);

        let network_size_bit_len = 0;
        let token_repr_bit_len = 10;
        let threshold = activity_threshold(network_size_bit_len, token_repr_bit_len);
        // 10 - 0 - 1
        assert_eq!(threshold, 9);

        let network_size_bit_len = 7;
        let token_repr_bit_len = 0;
        let threshold = activity_threshold(network_size_bit_len, token_repr_bit_len);
        // 0 - 7 - 1 (by saturated_sub)
        assert_eq!(threshold, 0);
    }

    #[test]
    fn test_token_count_bit_len() {
        let total_core_quota = 10;
        let message_frequency_per_round = 1.0.try_into().unwrap();
        // ceil(log2(10 * 1.0 + 1))
        assert_eq!(
            token_count_bit_len(total_core_quota, message_frequency_per_round).unwrap(),
            4
        );

        let total_core_quota = 0;
        let message_frequency_per_round = 1.0.try_into().unwrap();
        // ceil(log2(0 * 1.0 + 1))
        assert_eq!(
            token_count_bit_len(total_core_quota, message_frequency_per_round).unwrap(),
            0
        );

        let total_core_quota = 10;
        let message_frequency_per_round = 0.0.try_into().unwrap();
        // ceil(log2(10 * 0.0 + 1))
        assert_eq!(
            token_count_bit_len(total_core_quota, message_frequency_per_round).unwrap(),
            0
        );
    }

    #[test]
    fn test_blending_token_hamming_distance() {
        let token = blending_token(1, 2);
        let token_count_byte_len = token_count_bit_len(10, 1.0.try_into().unwrap())
            .unwrap()
            .div_ceil(8);

        // Check the expected distance.
        let distance = token.hamming_distance(token_count_byte_len, SessionRandomness([3u8; 64]));
        assert_eq!(distance, 4);
    }

    #[test_log::test(test)]
    fn test_blending_tokens() {
        let total_core_quota = 30;
        let session_info = SessionInfo::new(
            1,
            SessionRandomness([1u8; 64]),
            1,
            total_core_quota,
            1.0.try_into().unwrap(),
        )
        .unwrap();
        let mut tokens = BlendingTokens::new(&session_info);
        assert!(tokens.current_session_tokens.tokens.is_empty());
        assert_eq!(
            tokens.current_session_randomness,
            session_info.session_randomness
        );
        assert!(tokens.previous_session_tokens.is_none());

        // Insert tokens as many as the total core quota,
        // so that one of them can be picked as an activity proof.
        for i in 0..total_core_quota {
            let i: u8 = i.try_into().unwrap();
            let token = blending_token(i, i);
            tokens.insert(token.clone());
            assert!(tokens.current_session_tokens.tokens.contains(&token));
        }
        assert!(tokens.previous_session_tokens.is_none());

        // Try to compute an activity proof, but None is returned.
        assert!(tokens.activity_proof().is_none());

        // Rotate to a new session.
        tokens.rotate_session(
            &SessionInfo::new(
                2,
                SessionRandomness([2u8; 64]),
                1,
                60,
                1.0.try_into().unwrap(),
            )
            .unwrap(),
        );
        // Check if the sessions have been rotated correctly.
        assert!(tokens.current_session_tokens.tokens.is_empty());
        assert_eq!(
            tokens.current_session_randomness,
            SessionRandomness([2u8; 64])
        );
        assert!(tokens.previous_session_tokens.is_some());

        // An activity proof should be generated.
        let candidates = tokens
            .previous_session_tokens
            .as_ref()
            .unwrap()
            .tokens
            .clone();
        let proof = tokens.activity_proof().unwrap();
        assert!(candidates.contains(&proof.token));

        // An activity proof should not be generated again,
        // since the previous session tokens have been consumed.
        assert!(tokens.activity_proof().is_none());
    }

    fn blending_token(proof_of_quota: u8, proof_of_selection: u8) -> BlendingToken {
        BlendingToken {
            proof_of_quota: ProofOfQuota::from_bytes_unchecked(
                [proof_of_quota; PROOF_OF_QUOTA_SIZE],
            ),
            proof_of_selection: ProofOfSelection::from_bytes_unchecked(
                [proof_of_selection; PROOF_OF_SELECTION_SIZE],
            ),
        }
    }
}
