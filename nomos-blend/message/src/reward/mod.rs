mod activity;
mod session;
mod token;

use std::collections::HashSet;

pub use activity::ActivityProof;
use nomos_core::sdp::SessionNumber;
pub use session::SessionInfo;
pub use token::BlendingToken;
use tracing::warn;

use crate::reward::session::SessionRandomness;

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

    /// Computes an activity proof for the previous session by consuming
    /// blending tokens collected during that session.
    ///
    /// It returns `None` if there was no blending token collected during
    /// the previous session, or if there is no blending token satisfying the
    /// activity threshold.
    pub fn compute_activity_proof_for_previous_session(&mut self) -> Option<ActivityProof> {
        self.previous_session_tokens
            .take()?
            .compute_activity_proof(self.current_session_randomness)
    }

    /// Switches to the new session while retaining the current session tokens,
    /// so that new tokens for the session can continue to be collected during
    /// the session transition period.
    ///
    /// If there was a previous session that has not yet been consumed for
    /// an activity proof, it is discarded because its activity proof is
    /// no longer acceptable by the protocol.
    pub fn rotate_session(&mut self, new_session_info: &SessionInfo) {
        if self.previous_session_tokens.is_some() {
            warn!(
                target: LOG_TARGET,
                "Rotating to a new session while previous session tokens are still unconsumed. Those tokens will be discarded."
            );
        }

        self.previous_session_tokens = Some(std::mem::replace(
            &mut self.current_session_tokens,
            BlendingTokensForSession::new(
                new_session_info.session_number,
                new_session_info.token_count_byte_len,
                new_session_info.activity_threshold,
            ),
        ));
        self.current_session_randomness = new_session_info.session_randomness;
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
    fn compute_activity_proof(
        self,
        next_session_randomness: SessionRandomness,
    ) -> Option<ActivityProof> {
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
    use nomos_core::crypto::ZkHash;

    use super::*;
    use crate::crypto::proofs::{
        quota::{PROOF_OF_QUOTA_SIZE, ProofOfQuota},
        selection::{PROOF_OF_SELECTION_SIZE, ProofOfSelection},
    };

    #[test_log::test(test)]
    fn test_blending_token_collector() {
        let num_core_nodes = 2;
        let core_quota = 15;
        let session_info = SessionInfo::new(
            1,
            &ZkHash::from(1),
            num_core_nodes,
            core_quota,
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
        for i in 0..(core_quota.checked_mul(num_core_nodes).unwrap()) {
            let i: u8 = i.try_into().unwrap();
            let token = blending_token(i, i);
            tokens.insert(token.clone());
            assert!(tokens.current_session_tokens.tokens.contains(&token));
        }
        assert!(tokens.previous_session_tokens.is_none());

        // Try to compute an activity proof, but None is returned.
        assert!(
            tokens
                .compute_activity_proof_for_previous_session()
                .is_none()
        );

        // Rotate to a new session.
        let session_info = SessionInfo::new(
            2,
            &ZkHash::from(2),
            num_core_nodes,
            core_quota,
            1.0.try_into().unwrap(),
        )
        .unwrap();
        tokens.rotate_session(&session_info);
        // Check if the sessions have been rotated correctly.
        assert!(tokens.current_session_tokens.tokens.is_empty());
        assert_eq!(
            tokens.current_session_randomness,
            session_info.session_randomness
        );
        assert!(tokens.previous_session_tokens.is_some());

        // An activity proof should be generated.
        let candidates = tokens
            .previous_session_tokens
            .as_ref()
            .unwrap()
            .tokens
            .clone();
        let proof = tokens
            .compute_activity_proof_for_previous_session()
            .unwrap();
        assert!(candidates.contains(proof.token()));

        // An activity proof should not be generated again,
        // since the previous session tokens have been consumed.
        assert!(
            tokens
                .compute_activity_proof_for_previous_session()
                .is_none()
        );
    }

    fn blending_token(proof_of_quota: u8, proof_of_selection: u8) -> BlendingToken {
        BlendingToken::new(
            ProofOfQuota::from_bytes_unchecked([proof_of_quota; PROOF_OF_QUOTA_SIZE]),
            ProofOfSelection::from_bytes_unchecked([proof_of_selection; PROOF_OF_SELECTION_SIZE]),
        )
    }
}
