use std::{cmp::Ordering, collections::HashMap, iter::once};

use nomos_blend_message::reward::{BlendingTokenEvaluation, HammingDistance};
use nomos_blend_proofs::{quota::VerifiedProofOfQuota, selection::VerifiedProofOfSelection};
use nomos_core::{
    mantle::Utxo,
    sdp::{ProviderId, ServiceType, SessionNumber},
};
use rpds::{HashTrieMapSync, HashTrieSetSync};
use zksign::PublicKey;

use crate::mantle::sdp::rewards::{
    Error,
    blend::{RewardsParameters, current_session::CurrentSessionState},
    distribute_rewards,
};

/// The immutable state of the target session for which rewards are being
/// calculated. The target session is `s-1` if `s` is the current session.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetSessionState {
    /// The target session number
    session_number: SessionNumber,
    /// The providers in the target session
    providers: HashTrieMapSync<ProviderId, PublicKey>,
    /// Parameters for evaluating activity proofs in the target session
    token_evaluation: BlendingTokenEvaluation,
    // TODO: Add proof verifiers
}

impl TargetSessionState {
    pub const fn new(
        session_number: SessionNumber,
        providers: HashTrieMapSync<ProviderId, PublicKey>,
        token_evaluation: BlendingTokenEvaluation,
    ) -> Self {
        Self {
            session_number,
            providers,
            token_evaluation,
        }
    }

    pub const fn session_number(&self) -> SessionNumber {
        self.session_number
    }

    pub fn providers(&self) -> impl Iterator<Item = (&ProviderId, &PublicKey)> {
        self.providers.iter()
    }

    fn num_providers(&self) -> u64 {
        self.providers
            .size()
            .try_into()
            .expect("number of providers must fit in u64")
    }

    pub fn verify_proof(
        &self,
        provider_id: &ProviderId,
        proof: &nomos_core::sdp::blend::ActivityProof,
        current_session_state: &CurrentSessionState,
        settings: &RewardsParameters,
    ) -> Result<(PublicKey, HammingDistance), Error> {
        if proof.session != self.session_number {
            return Err(Error::InvalidSession {
                expected: self.session_number,
                got: proof.session,
            });
        }

        let num_providers = self.num_providers();
        assert!(
            num_providers >= settings.minimum_network_size.get(),
            "number of providers must be >= minimum_network_size"
        );
        let &zk_id = self
            .providers
            .get(provider_id)
            .ok_or_else(|| Error::UnknownProvider(Box::new(*provider_id)))?;

        let verified_proof = verify_activity_proof(proof);

        let Some(hamming_distance) = self.token_evaluation.evaluate(
            verified_proof.token(),
            current_session_state.session_randomness(),
        ) else {
            return Err(Error::InvalidProof);
        };

        Ok((zk_id, hamming_distance))
    }
}

fn verify_activity_proof(
    proof: &nomos_core::sdp::blend::ActivityProof,
) -> nomos_blend_message::reward::ActivityProof {
    // TODO: Verify PoQ and PoSel by holding all epoch infos that
    // spanned across the target session.
    // For now, we just accept them without verification.
    let verified_proof_of_quota =
        VerifiedProofOfQuota::from_bytes_unchecked((&proof.proof_of_quota).into());
    let verified_proof_of_selection =
        VerifiedProofOfSelection::from_bytes_unchecked((&proof.proof_of_selection).into());
    nomos_blend_message::reward::ActivityProof::new(
        proof.session,
        nomos_blend_message::reward::BlendingToken::new(
            verified_proof_of_quota,
            verified_proof_of_selection,
        ),
    )
}

/// Tracks activity proofs submitted for the target session whose rewards are
/// being calculated. The target session is `s-1` if `s` is the current session.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetSessionTracker {
    /// Collecting proofs submitted by providers in the target session.
    submitted_proofs: HashTrieMapSync<ProviderId, (PublicKey, HammingDistance)>,
    /// Tracking the minimum Hamming distance among submitted proofs.
    min_hamming_distance: MinHammingDistance,
}

impl Default for TargetSessionTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl TargetSessionTracker {
    pub fn new() -> Self {
        Self {
            submitted_proofs: HashTrieMapSync::new_sync(),
            min_hamming_distance: MinHammingDistance::new(),
        }
    }

    pub fn insert(
        &self,
        provider_id: ProviderId,
        session: SessionNumber,
        zk_id: PublicKey,
        hamming_distance: HammingDistance,
    ) -> Result<Self, Error> {
        if self.submitted_proofs.contains_key(&provider_id) {
            return Err(Error::DuplicateActiveMessage {
                session,
                provider_id: Box::new(provider_id),
            });
        }
        Ok(Self {
            submitted_proofs: self
                .submitted_proofs
                .insert(provider_id, (zk_id, hamming_distance)),
            min_hamming_distance: self
                .min_hamming_distance
                .with_update(hamming_distance, provider_id),
        })
    }

    pub fn finalize(
        &self,
        session_number: SessionNumber,
        session_income: u64,
    ) -> (Self, Vec<Utxo>) {
        // Identify premium providers with the minimum Hamming distance
        let premium_providers = &self.min_hamming_distance.providers;

        // Calculate base reward
        let base_reward = if self.submitted_proofs.is_empty() {
            0
        } else {
            session_income / (self.submitted_proofs.size() as u64 + premium_providers.size() as u64)
        };

        // Calculate reward for each provider
        let mut rewards = HashMap::new();
        for (provider_id, (zk_id, _)) in self.submitted_proofs.iter() {
            let reward = if premium_providers.contains(provider_id) {
                base_reward * 2
            } else {
                base_reward
            };
            rewards.insert(*zk_id, reward);
        }

        (
            Self::new(),
            distribute_rewards(rewards, session_number, ServiceType::BlendNetwork),
        )
    }
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MinHammingDistance {
    min_distance: HammingDistance,
    providers: HashTrieSetSync<ProviderId>,
}

impl MinHammingDistance {
    fn new() -> Self {
        Self {
            min_distance: HammingDistance::MAX,
            providers: HashTrieSetSync::new_sync(),
        }
    }

    /// Creates a new [`MinHammingDistance`] updated with the given distance and
    /// provider.
    fn with_update(&self, distance: HammingDistance, provider: ProviderId) -> Self {
        match distance.cmp(&self.min_distance) {
            Ordering::Less => Self {
                min_distance: distance,
                providers: once(provider).collect(),
            },
            Ordering::Equal => Self {
                min_distance: distance,
                providers: self.providers.insert(provider),
            },
            Ordering::Greater => self.clone(),
        }
    }
}
