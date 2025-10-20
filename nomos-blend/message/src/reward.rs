use std::{collections::HashSet, ops::Deref};

use blake2::{
    Blake2bVar,
    digest::{Update as _, VariableOutput as _},
};
use nomos_core::{block::SessionNumber, codec::SerializeOp as _};
use nomos_utils::math::{F64Ge1, NonNegativeF64};
use serde::Serialize;
use tracing::{debug, error};

use crate::crypto::proofs::{quota::ProofOfQuota, selection::ProofOfSelection};

const LOG_TARGET: &str = "blend::message::reward";

/// Blending tokens collected during the current session.
///
/// This also holds the blending tokens collected from the previous session
/// to produce an activity proof for that session after the session transition
/// period has passed.
pub struct BlendingTokens {
    current_session_tokens: SessionBlendingTokens,
    current_session_randomness: SessionRandomness,
    previous_session_tokens: Option<SessionBlendingTokens>,
}

impl BlendingTokens {
    #[must_use]
    pub fn new(current_session_info: &SessionInfo) -> Self {
        Self {
            current_session_tokens: SessionBlendingTokens::new(
                current_session_info.session_number,
                current_session_info.num_core_nodes,
                current_session_info.total_core_quota,
                current_session_info.message_frequency_per_round,
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
    /// It returns `Ok(None)` if there was no blending token collected during
    /// the previous session, or if there is no blending token satisfying the
    /// activity threshold.
    /// It returns `Err` if the computation fails.
    pub fn activity_proof(&mut self) -> Result<Option<ActivityProof>, Error> {
        let Some(prev_session_tokens) = self.previous_session_tokens.take() else {
            return Ok(None);
        };
        prev_session_tokens.activity_proof(self.current_session_randomness)
    }

    /// Switches to the new session, keeping the current session tokens
    /// for the session transition.
    ///
    /// If there was a previous session that has not yet been consumed for
    /// an activity proof, it is discarded.
    pub fn rotate_session(&mut self, new_session_info: &SessionInfo) {
        self.previous_session_tokens = Some(std::mem::replace(
            &mut self.current_session_tokens,
            SessionBlendingTokens::new(
                new_session_info.session_number,
                new_session_info.num_core_nodes,
                new_session_info.total_core_quota,
                new_session_info.message_frequency_per_round,
            ),
        ));
        self.current_session_randomness = new_session_info.session_randomness;
    }
}

/// Arguments required to build [`BlendingTokens`].
pub struct SessionInfo {
    session_number: SessionNumber,
    session_randomness: SessionRandomness,
    num_core_nodes: u64,
    total_core_quota: u64,
    message_frequency_per_round: NonNegativeF64,
}

/// Blending tokens collected during a single session.
struct SessionBlendingTokens {
    session_number: SessionNumber,
    num_core_nodes: u64,
    total_core_quota: u64,
    message_frequency_per_round: NonNegativeF64,
    tokens: HashSet<BlendingToken>,
}

impl SessionBlendingTokens {
    fn new(
        session_number: SessionNumber,
        num_core_nodes: u64,
        total_core_quota: u64,
        message_frequency_per_round: NonNegativeF64,
    ) -> Self {
        Self {
            session_number,
            num_core_nodes,
            total_core_quota,
            message_frequency_per_round,
            tokens: HashSet::new(),
        }
    }

    fn insert(&mut self, token: BlendingToken) {
        self.tokens.insert(token);
    }

    /// Computes an activity proof for this session by consuming tokens.
    ///
    /// It returns `Ok(None)` if there was no blending token satisfying the
    /// activity threshold calculated.
    /// It returns `Err` if the computation fails.
    fn activity_proof(
        self,
        next_session_randomness: SessionRandomness,
    ) -> Result<Option<ActivityProof>, Error> {
        // First, compute the activity threshold.
        let activity_threshold = activity_threshold(
            self.num_core_nodes,
            BlendingToken::repr_bit_len(self.total_core_quota, self.message_frequency_per_round)?,
        )?;
        debug!(
            target: LOG_TARGET,
            "Activity threshold for session {}: {}",
            self.session_number, activity_threshold
        );

        // Filter tokens whose Hamming distance is below the activity threshold,\
        // and pick the one with the smallest distance.
        let maybe_token = self
            .tokens
            .into_iter()
            .filter_map(|token| {
                match token.hamming_distance(
                    self.message_frequency_per_round,
                    self.total_core_quota,
                    next_session_randomness,
                ) {
                    Ok(distance) => {
                        debug!(
                            target: LOG_TARGET,
                            "Token distance:{distance}, threshold: {activity_threshold}"
                        );
                        (distance <= activity_threshold).then_some((token, distance))
                    }
                    Err(e) => {
                        error!(target: LOG_TARGET, "Failed to compute Hamming distance: {e}");
                        None
                    }
                }
            })
            .min_by_key(|(_, distance)| *distance);
        let Some((token, _)) = maybe_token else {
            return Ok(None);
        };
        Ok(Some(ActivityProof {
            session_number: self.session_number,
            token,
        }))
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
        message_frequency_per_round: NonNegativeF64,
        total_core_quota: u64,
        next_session_randomness: SessionRandomness,
    ) -> Result<u64, Error> {
        let token = self
            .to_bytes()
            .expect("BlendingToken should be serializable");
        let token_repr_byte_len =
            Self::repr_byte_len(total_core_quota, message_frequency_per_round)? as usize;

        let token_hash = hash(&token, token_repr_byte_len);
        let session_randomness_hash = hash(&next_session_randomness, token_repr_byte_len);

        Ok(hamming_distance(&token_hash, &session_randomness_hash))
    }

    /// The number of bytes that can represent the maximum number of blending
    /// tokens generated during a single session.
    fn repr_byte_len(
        total_core_quota: u64,
        message_frequency_per_round: NonNegativeF64,
    ) -> Result<u64, Error> {
        let bit_len = Self::repr_bit_len(total_core_quota, message_frequency_per_round)?;
        Ok(bit_len.div_ceil(8))
    }

    /// The number of bits that can represent the maximum number of blending
    /// tokens generated during a single session.
    fn repr_bit_len(
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
fn activity_threshold(num_core_nodes: u64, token_repr_bit_len: u64) -> Result<u64, Error> {
    debug!(
        target: LOG_TARGET,
        "Calculating activity threshold: num_core_nodes={num_core_nodes}, token_repr_bit_len={token_repr_bit_len}"
    );

    // Calculate the lottery difficulty (the number of bits that can represent the
    // number of core nodes in the network).
    let network_size_repr_bit_len = F64Ge1::try_from(
        num_core_nodes
            .checked_add(1)
            .ok_or(Error::NetworkSizeTooLarge(num_core_nodes))?,
    )
    .map_err(|()| Error::NetworkSizeTooLarge(num_core_nodes))?
    .log2()
    .ceil() as u64;

    Ok(token_repr_bit_len
        .saturating_sub(network_size_repr_bit_len)
        .saturating_sub(ACTIVITY_THRESHOLD_SENSITIVITY_PARAM))
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
        let num_core_nodes = 100;
        let token_repr_bit_len = 10;
        let threshold = activity_threshold(num_core_nodes, token_repr_bit_len);
        // 10 - ceil(log2(100+1)) - 1
        assert_eq!(threshold.unwrap(), 2);

        let num_core_nodes = 0;
        let token_repr_bit_len = 10;
        let threshold = activity_threshold(num_core_nodes, token_repr_bit_len);
        // 10 - ceil(log2(0+1)) - 1
        assert_eq!(threshold.unwrap(), 9);

        let num_core_nodes = 100;
        let token_repr_bit_len = 0;
        let threshold = activity_threshold(num_core_nodes, token_repr_bit_len);
        // 0 - ceil(log2(100+1)) - 1 (by saturated_sub)
        assert_eq!(threshold.unwrap(), 0);

        let num_core_nodes = u64::MAX;
        let token_repr_bit_len = 10;
        let threshold = activity_threshold(num_core_nodes, token_repr_bit_len);
        assert!(matches!(
            threshold.unwrap_err(),
            Error::NetworkSizeTooLarge(_)
        ));

        let num_core_nodes = u64::MAX;
        let token_repr_bit_len = 10;
        let threshold = activity_threshold(num_core_nodes, token_repr_bit_len);
        assert!(matches!(
            threshold.unwrap_err(),
            Error::NetworkSizeTooLarge(_)
        ));
    }

    #[test]
    fn test_blending_token_repr_bit_len() {
        let total_core_quota = 10;
        let message_frequency_per_round = 1.0.try_into().unwrap();
        let bit_len =
            BlendingToken::repr_bit_len(total_core_quota, message_frequency_per_round).unwrap();
        let byte_len =
            BlendingToken::repr_byte_len(total_core_quota, message_frequency_per_round).unwrap();
        // ceil(log2(10 * 1.0 + 1))
        assert_eq!(bit_len, 4);
        assert_eq!(byte_len, 1);

        let total_core_quota = 0;
        let message_frequency_per_round = 1.0.try_into().unwrap();
        let bit_len =
            BlendingToken::repr_bit_len(total_core_quota, message_frequency_per_round).unwrap();
        let byte_len =
            BlendingToken::repr_byte_len(total_core_quota, message_frequency_per_round).unwrap();
        // ceil(log2(0 * 1.0 + 1))
        assert_eq!(bit_len, 0);
        assert_eq!(byte_len, 0);

        let total_core_quota = 10;
        let message_frequency_per_round = 0.0.try_into().unwrap();
        let bit_len =
            BlendingToken::repr_bit_len(total_core_quota, message_frequency_per_round).unwrap();
        let byte_len =
            BlendingToken::repr_byte_len(total_core_quota, message_frequency_per_round).unwrap();
        // ceil(log2(1 * 0.0 + 1))
        assert_eq!(bit_len, 0);
        assert_eq!(byte_len, 0);

        let total_core_quota = u64::MAX;
        let message_frequency_per_round = 1.0.try_into().unwrap();
        // ceil(log2(TOO_LARGE * 1.0 + 1))
        assert!(matches!(
            BlendingToken::repr_bit_len(total_core_quota, message_frequency_per_round),
            Err(Error::TotalCoreQuotaTooLarge(_))
        ));
        assert!(matches!(
            BlendingToken::repr_byte_len(total_core_quota, message_frequency_per_round),
            Err(Error::TotalCoreQuotaTooLarge(_))
        ));
    }

    #[test]
    fn test_blending_token_hamming_distance() {
        let token = blending_token(1, 2);

        // Check the expected distance.
        let distance = token
            .hamming_distance(1.0.try_into().unwrap(), 10, SessionRandomness([3u8; 64]))
            .unwrap();
        assert_eq!(distance, 4);

        // Error if total_core_quota is too large.
        assert!(matches!(
            token.hamming_distance(
                1.0.try_into().unwrap(),
                u64::MAX,
                SessionRandomness([3u8; 64])
            ),
            Err(Error::TotalCoreQuotaTooLarge(_))
        ));
    }

    #[test_log::test(test)]
    fn test_blending_tokens() {
        let session_info = SessionInfo {
            session_number: 1,
            session_randomness: SessionRandomness([1u8; 64]),
            num_core_nodes: 1,
            total_core_quota: 30,
            message_frequency_per_round: 1.0.try_into().unwrap(),
        };
        let mut tokens = BlendingTokens::new(&session_info);
        assert!(tokens.current_session_tokens.tokens.is_empty());
        assert_eq!(
            tokens.current_session_randomness,
            session_info.session_randomness
        );
        assert!(tokens.previous_session_tokens.is_none());

        // Insert tokens as many as the total core quota,
        // so that one of them can be picked as an activity proof.
        for i in 0..session_info.total_core_quota {
            let i: u8 = i.try_into().unwrap();
            let token = blending_token(i, i);
            tokens.insert(token.clone());
            assert!(tokens.current_session_tokens.tokens.contains(&token));
        }
        assert!(tokens.previous_session_tokens.is_none());

        // Try to compute an activity proof, but None is returned.
        assert!(matches!(tokens.activity_proof(), Ok(None)));

        // Rotate to a new session.
        tokens.rotate_session(&SessionInfo {
            session_number: 2,
            session_randomness: SessionRandomness([2u8; 64]),
            num_core_nodes: 1,
            total_core_quota: 60,
            message_frequency_per_round: 1.0.try_into().unwrap(),
        });
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
        let proof = tokens.activity_proof().unwrap().unwrap();
        assert!(candidates.contains(&proof.token));

        // An activity proof should not be generated again,
        // since the previous session tokens have been consumed.
        assert!(matches!(tokens.activity_proof(), Ok(None)));
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
