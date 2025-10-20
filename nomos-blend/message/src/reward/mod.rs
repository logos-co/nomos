mod activity;
mod session;
mod token;

use std::collections::HashSet;

pub use activity::ActivityProof;
use nomos_core::sdp::SessionNumber;
pub use session::{SessionInfo, SessionRandomness};
pub use token::BlendingToken;

const LOG_TARGET: &str = "blend::message::reward";

/// Collects blending tokens for the current session, while retaining those from
/// the previous session which are used to produce an activity proof.
pub struct BlendingTokenCollector {
    current_session_tokens: BlendingTokensForSession,
    current_session_randomness: SessionRandomness,
    previous_session_tokens: Option<BlendingTokensForSession>,
}

impl BlendingTokenCollector {
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
            Some(ActivityProof::new(self.session_number, token))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::proofs::{
        quota::{PROOF_OF_QUOTA_SIZE, ProofOfQuota},
        selection::{PROOF_OF_SELECTION_SIZE, ProofOfSelection},
    };

    #[test_log::test(test)]
    fn test_blending_token_collector() {
        let total_core_quota = 30;
        let session_info = SessionInfo::new(
            1,
            [1u8; 64].into(),
            1,
            total_core_quota,
            1.0.try_into().unwrap(),
        )
        .unwrap();
        let mut tokens = BlendingTokenCollector::new(&session_info);
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
            &SessionInfo::new(2, [2u8; 64].into(), 1, 60, 1.0.try_into().unwrap()).unwrap(),
        );
        // Check if the sessions have been rotated correctly.
        assert!(tokens.current_session_tokens.tokens.is_empty());
        assert_eq!(tokens.current_session_randomness, [2u8; 64].into());
        assert!(tokens.previous_session_tokens.is_some());

        // An activity proof should be generated.
        let candidates = tokens
            .previous_session_tokens
            .as_ref()
            .unwrap()
            .tokens
            .clone();
        let proof = tokens.activity_proof().unwrap();
        assert!(candidates.contains(proof.token()));

        // An activity proof should not be generated again,
        // since the previous session tokens have been consumed.
        assert!(tokens.activity_proof().is_none());
    }

    fn blending_token(proof_of_quota: u8, proof_of_selection: u8) -> BlendingToken {
        BlendingToken::new(
            ProofOfQuota::from_bytes_unchecked([proof_of_quota; PROOF_OF_QUOTA_SIZE]),
            ProofOfSelection::from_bytes_unchecked([proof_of_selection; PROOF_OF_SELECTION_SIZE]),
        )
    }
}
