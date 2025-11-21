mod session;

use std::collections::HashSet;

use nomos_core::{
    blend::{BlendingToken, SessionRandomness},
    sdp::{BlendActivityProof, SessionNumber},
};
pub use session::SessionInfo;

const LOG_TARGET: &str = "blend::message::reward";

pub trait BlendingTokenCollector {
    fn collect(&mut self, token: BlendingToken);
}

/// Holds blending tokens collected during a single session.
pub struct SessionBlendingTokenCollector {
    session_number: SessionNumber,
    token_count_byte_len: u64,
    activity_threshold: u64,
    tokens: HashSet<BlendingToken>,
}

impl SessionBlendingTokenCollector {
    #[must_use]
    pub fn new(session_info: &SessionInfo) -> Self {
        Self {
            session_number: session_info.session_number,
            token_count_byte_len: session_info.token_count_byte_len,
            activity_threshold: session_info.activity_threshold,
            tokens: HashSet::new(),
        }
    }

    #[must_use]
    pub fn rotate_session(
        self,
        new_session_info: &SessionInfo,
    ) -> (Self, OldSessionBlendingTokenCollector) {
        let new_collector = Self::new(new_session_info);
        let old_collector = OldSessionBlendingTokenCollector {
            collector: self,
            next_session_randomness: new_session_info.session_randomness(),
        };
        (new_collector, old_collector)
    }

    #[cfg(test)]
    #[must_use]
    const fn tokens(&self) -> &HashSet<BlendingToken> {
        &self.tokens
    }
}

impl BlendingTokenCollector for SessionBlendingTokenCollector {
    fn collect(&mut self, token: BlendingToken) {
        self.tokens.insert(token);
    }
}

/// Holds blending tokens collected during the old session.
pub struct OldSessionBlendingTokenCollector {
    collector: SessionBlendingTokenCollector,
    next_session_randomness: SessionRandomness,
}

impl OldSessionBlendingTokenCollector {
    pub fn collect(&mut self, token: BlendingToken) {
        self.collector.collect(token);
    }

    /// Computes an activity proof for this session by consuming tokens.
    ///
    /// It returns `None` if there was no blending token satisfying the
    /// activity threshold calculated.
    #[must_use]
    pub fn compute_activity_proof(self) -> Option<BlendActivityProof> {
        // Find the blending token with the smallest Hamming distance.
        let maybe_token = self
            .collector
            .tokens
            .into_iter()
            .map(|token| {
                let distance = token.hamming_distance(
                    self.collector.token_count_byte_len,
                    self.next_session_randomness,
                );
                (token, distance)
            })
            .min_by_key(|(_, distance)| *distance);

        // Check if the smallest distance satisfies the activity threshold.
        if let Some((token, distance)) = maybe_token
            && distance <= self.collector.activity_threshold
        {
            Some(BlendActivityProof::new(
                self.collector.session_number,
                token,
            ))
        } else {
            None
        }
    }

    #[cfg(test)]
    #[must_use]
    const fn tokens(&self) -> &HashSet<BlendingToken> {
        self.collector.tokens()
    }
}

impl BlendingTokenCollector for OldSessionBlendingTokenCollector {
    fn collect(&mut self, token: BlendingToken) {
        self.collector.collect(token);
    }
}

#[cfg(test)]
mod tests {
    use nomos_core::{
        blend::{PROOF_OF_QUOTA_SIZE, PROOF_OF_SELECTION_SIZE, ProofOfQuota, ProofOfSelection},
        crypto::ZkHash,
    };

    use super::*;

    #[test_log::test(test)]
    fn test_blending_token_collector() {
        let num_core_nodes = 2;
        let core_quota = 15;
        let session_info =
            SessionInfo::new(1, &ZkHash::from(1), num_core_nodes, core_quota).unwrap();
        let mut tokens = SessionBlendingTokenCollector::new(&session_info);
        assert!(tokens.tokens().is_empty());

        // Insert `total_core_quota-1` tokens.
        let total_core_quota = core_quota.checked_mul(num_core_nodes).unwrap();
        let mut i = 0;
        for _ in 0..(total_core_quota.checked_sub(1).unwrap()) {
            let proof: u8 = i.try_into().unwrap();
            let token = blending_token(proof, proof);
            tokens.collect(token.clone());
            assert!(tokens.tokens().contains(&token));
            i += 1;
        }

        // Prepare a new session info.
        let session_info =
            SessionInfo::new(2, &ZkHash::from(2), num_core_nodes, core_quota).unwrap();
        let (_, mut tokens) = tokens.rotate_session(&session_info);

        // Insert one more tokens.
        // Now,`total_core_quota` tokens have been collected.
        // So, we can expect that always one of them can be picked as an activity proof.
        let proof: u8 = i.try_into().unwrap();
        let token = blending_token(proof, proof);
        tokens.collect(token.clone());
        assert!(tokens.tokens().contains(&token));

        // Compute an activity proof
        let candidates = tokens.tokens().clone();
        let proof = tokens.compute_activity_proof().unwrap();
        assert!(candidates.contains(proof.token()));
    }

    fn blending_token(proof_of_quota: u8, proof_of_selection: u8) -> BlendingToken {
        BlendingToken::new(
            ProofOfQuota::from_bytes_unchecked([proof_of_quota; PROOF_OF_QUOTA_SIZE]),
            ProofOfSelection::from_bytes_unchecked([proof_of_selection; PROOF_OF_SELECTION_SIZE]),
        )
    }
}
