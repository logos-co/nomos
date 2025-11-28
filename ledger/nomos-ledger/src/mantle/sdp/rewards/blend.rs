use std::{cmp::Ordering, collections::HashMap, iter::once, num::NonZeroU64};

use groth16::Fr;
use nomos_blend_message::reward::{BlendingTokenEvaluation, SessionRandomness};
use nomos_blend_proofs::{quota::VerifiedProofOfQuota, selection::VerifiedProofOfSelection};
use nomos_core::{
    blend::core_quota,
    block::BlockNumber,
    mantle::Utxo,
    sdp::{ActivityMetadata, ProviderId, ServiceParameters, ServiceType, SessionNumber},
};
use nomos_utils::math::NonNegativeF64;
use rpds::{HashTrieMapSync, HashTrieSetSync};
use zksign::PublicKey;

use crate::mantle::sdp::{
    SessionState,
    rewards::{Error, distribute_rewards},
};

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq)]
pub enum Rewards {
    /// State before the first session update (0 -> 1).
    /// No activity messages are accepted in this state, because activity cannot
    /// exist before session 0.
    Uninitialized(RewardsParameters),
    /// State after the first session update (0 -> 1).
    /// This is updated every new session.
    Initialized {
        /// The session that is referenced by the submitted proofs.
        /// This is `s-1` if `s` is the session at which this message was sent.
        session_number: SessionNumber,
        declarations: HashTrieMapSync<ProviderId, PublicKey>,
        /// Parameters for evaluating activity proofs in the session
        token_evaluation: BlendingTokenEvaluation,
        /// Session randomness for the session `s`.
        next_session_randomness: SessionRandomness,
        /// Proofs submitted by providers in the session corresponding to
        /// `session_number`.
        submitted_proofs: HashTrieMapSync<ProviderId, (PublicKey, u64)>,
        /// Tracking the minimum Hamming distance among submitted proofs.
        min_hamming_distance: MinHammingDistance,
        /// Settings that don't change per session.
        settings: RewardsParameters,
    },
}

impl Rewards {
    /// Create a new uninitialized [`Rewards`] that doesn't accept activity
    /// messages until the first session update.
    #[must_use]
    pub const fn new(settings: RewardsParameters) -> Self {
        Self::Uninitialized(settings)
    }
}

impl super::Rewards for Rewards {
    fn update_active(
        &self,
        provider_id: ProviderId,
        metadata: &ActivityMetadata,
        _block_number: BlockNumber,
    ) -> Result<Self, Error> {
        match self {
            Self::Uninitialized(_) => {
                // Reject all activity messages.
                Err(Error::Uninitialized)
            }
            Self::Initialized {
                session_number,
                declarations,
                token_evaluation,
                next_session_randomness,
                submitted_proofs,
                min_hamming_distance,
                settings,
            } => {
                let ActivityMetadata::Blend(proof) = metadata else {
                    return Err(Error::InvalidProofType);
                };

                if submitted_proofs.contains_key(&provider_id) {
                    return Err(Error::DuplicateActiveMessage {
                        session: proof.session,
                        provider_id: Box::new(provider_id),
                    });
                }

                if proof.session != *session_number {
                    return Err(Error::InvalidSession {
                        expected: *session_number,
                        got: proof.session,
                    });
                }

                let num_declarations = declarations.size() as u64;
                if num_declarations < settings.minimum_network_size.get() {
                    return Err(Error::MinimumNetworkSizeNotSatisfied {
                        num_declarations,
                        minimum_network_size: settings.minimum_network_size,
                    });
                }
                let zk_id = declarations
                    .get(&provider_id)
                    .ok_or_else(|| Error::UnknownProvider(Box::new(provider_id)))?;

                let proof = verify_activity_proof(proof);

                let Some(hamming_distance) =
                    token_evaluation.evaluate(proof.token(), *next_session_randomness)
                else {
                    return Err(Error::InvalidProof);
                };

                Ok(Self::Initialized {
                    session_number: *session_number,
                    declarations: declarations.clone(),
                    token_evaluation: *token_evaluation,
                    next_session_randomness: *next_session_randomness,
                    submitted_proofs: submitted_proofs
                        .insert(provider_id, (*zk_id, hamming_distance)),
                    min_hamming_distance: min_hamming_distance
                        .with_update(hamming_distance, provider_id),
                    settings: settings.clone(),
                })
            }
        }
    }

    fn update_session(
        &self,
        last_active: &SessionState,
        next_active_session_epoch_nonce: &Fr,
        _config: &ServiceParameters,
    ) -> (Self, Vec<Utxo>) {
        match self {
            Self::Uninitialized(settings) => {
                // Prepare the proof evaluation parameters for the new session
                let token_evaluation = settings.token_evaluation(
                    last_active.declarations.size() as u64,
                ).expect("evaluation parameters shouldn't overflow. panicking since we can't process the new session");

                (
                    Self::Initialized {
                        session_number: last_active.session_n,
                        declarations: build_declaration_map(last_active),
                        token_evaluation,
                        next_session_randomness: SessionRandomness::new(
                            last_active.session_n + 1,
                            next_active_session_epoch_nonce,
                        ),
                        submitted_proofs: HashTrieMapSync::new_sync(),
                        min_hamming_distance: MinHammingDistance::new(),
                        settings: settings.clone(),
                    },
                    Vec::new(),
                )
            }
            Self::Initialized {
                submitted_proofs,
                min_hamming_distance,
                settings,
                ..
            } => {
                // TODO: Calculate base rewards when session_income is added to config
                // For now using placeholder value of 0
                let session_income = 0;

                // Identify premium providers with the minimum Hamming distance
                let premium_providers = &min_hamming_distance.providers;

                // Calculate base reward
                let base_reward = if submitted_proofs.is_empty() {
                    0
                } else {
                    session_income
                        / (submitted_proofs.size() as u64 + premium_providers.size() as u64)
                };

                // Calculate reward for each provider
                let mut rewards = HashMap::new();
                for (provider_id, (zk_id, _)) in submitted_proofs {
                    let reward = if premium_providers.contains(provider_id) {
                        base_reward * 2
                    } else {
                        base_reward
                    };
                    rewards.insert(*zk_id, reward);
                }

                // Prepare the proof evaluation parameters for the new session
                let token_evaluation = settings.token_evaluation(
                    last_active.declarations.size() as u64,
                ).expect("evaluation parameters shouldn't overflow. panicking since we can't process the new session");

                // Create new rewards state with updated sessions
                let new_state = Self::Initialized {
                    session_number: last_active.session_n,
                    declarations: build_declaration_map(last_active),
                    token_evaluation,
                    next_session_randomness: SessionRandomness::new(
                        last_active.session_n + 1,
                        next_active_session_epoch_nonce,
                    ),
                    submitted_proofs: HashTrieMapSync::new_sync(),
                    min_hamming_distance: MinHammingDistance::new(),
                    settings: settings.clone(),
                };

                (
                    new_state,
                    distribute_rewards(rewards, last_active.session_n, ServiceType::BlendNetwork),
                )
            }
        }
    }
}

fn build_declaration_map(session_state: &SessionState) -> HashTrieMapSync<ProviderId, PublicKey> {
    session_state
        .declarations
        .values()
        .map(|declaration| (declaration.provider_id, declaration.zk_id))
        .collect()
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

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct RewardsParameters {
    pub rounds_per_session: NonZeroU64,
    pub message_frequency_per_round: NonNegativeF64,
    pub num_blend_layers: NonZeroU64,
    pub minimum_network_size: NonZeroU64,
}

impl RewardsParameters {
    fn token_evaluation(
        &self,
        num_core_nodes: u64,
    ) -> Result<BlendingTokenEvaluation, nomos_blend_message::reward::Error> {
        let core_quota = core_quota(
            self.rounds_per_session,
            self.message_frequency_per_round,
            self.num_blend_layers,
            num_core_nodes as usize,
        );
        BlendingTokenEvaluation::new(core_quota, num_core_nodes)
    }
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MinHammingDistance {
    distance: u64,
    providers: HashTrieSetSync<ProviderId>,
}

impl MinHammingDistance {
    fn new() -> Self {
        Self {
            distance: u64::MAX,
            providers: HashTrieSetSync::new_sync(),
        }
    }

    /// Creates a new [`MinHammingDistance`] updated with the given distance and
    /// provider.
    fn with_update(&self, distance: u64, provider: ProviderId) -> Self {
        match distance.cmp(&self.distance) {
            Ordering::Less => Self {
                distance,
                providers: once(provider).collect(),
            },
            Ordering::Equal => Self {
                distance,
                providers: self.providers.insert(provider),
            },
            Ordering::Greater => self.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use groth16::Field as _;
    use nomos_blend_proofs::{quota::ProofOfQuota, selection::ProofOfSelection};
    use nomos_core::{crypto::ZkHash, sdp::blend};
    use num_bigint::BigUint;

    use super::*;
    use crate::mantle::sdp::rewards::{
        Rewards as _,
        tests::{create_provider_id, create_service_parameters, create_test_session_state},
    };

    fn create_blend_rewards_params(
        rounds_per_session: u64,
        minimum_network_size: u64,
    ) -> RewardsParameters {
        RewardsParameters {
            rounds_per_session: rounds_per_session.try_into().unwrap(),
            message_frequency_per_round: NonNegativeF64::try_from(1.0).unwrap(),
            num_blend_layers: NonZeroU64::new(3).unwrap(),
            minimum_network_size: minimum_network_size.try_into().unwrap(),
        }
    }

    fn new_proof_of_quota_unchecked(byte: u8) -> ProofOfQuota {
        VerifiedProofOfQuota::from_bytes_unchecked([byte; _]).into()
    }

    fn new_proof_of_selection_unchecked(byte: u8) -> ProofOfSelection {
        VerifiedProofOfSelection::from_bytes_unchecked([byte; _]).into()
    }

    #[test]
    fn test_blend_no_reward_calculated_after_session_0() {
        // Create a reward tracker
        let rewards_tracker = Rewards::new(create_blend_rewards_params(864_000, 1));

        // Create session_0 with providers
        let session_0 = create_test_session_state(
            &[create_provider_id(1), create_provider_id(2)],
            ServiceType::BlendNetwork,
            0,
        );

        // Update session from 0 to 1
        let (_, rewards) =
            rewards_tracker.update_session(&session_0, &ZkHash::ZERO, &create_service_parameters());

        // No rewards should be returned yet because session0 just ended,
        // and the reward calculation for the session0 just began.
        assert_eq!(rewards.len(), 0);
    }

    #[test]
    fn test_rewards_with_no_activity_proofs() {
        // Create a reward tracker, and update session from 0 to 1.
        let config = create_service_parameters();
        let (rewards_tracker, _) = Rewards::new(create_blend_rewards_params(864_000, 1))
            .update_session(
                &create_test_session_state(
                    &[create_provider_id(1), create_provider_id(2)],
                    ServiceType::BlendNetwork,
                    0,
                ),
                &ZkHash::ZERO,
                &config,
            );

        // Update session from 1 to 2 without any activity proofs submitted.
        let (_, rewards) = rewards_tracker.update_session(
            &create_test_session_state(
                &[create_provider_id(1), create_provider_id(2)],
                ServiceType::BlendNetwork,
                1,
            ),
            &ZkHash::ZERO,
            &config,
        );
        assert_eq!(rewards.len(), 0);
    }

    #[test]
    #[ignore = "TODO: Re-enable when session_income is implemented (currently hardcoded to 0)"]
    fn test_rewards_calculation() {
        let provider1 = create_provider_id(1);
        let provider2 = create_provider_id(2);
        let provider3 = create_provider_id(3);
        let provider4 = create_provider_id(4);

        // Create a reward tracker, and update session from 0 to 1.
        let config = create_service_parameters();
        let (rewards_tracker, _) = Rewards::new(create_blend_rewards_params(864_000, 1))
            .update_session(
                &create_test_session_state(
                    &[provider1, provider2, provider3, provider4],
                    ServiceType::BlendNetwork,
                    0,
                ),
                &ZkHash::ZERO,
                &config,
            );

        // provider1 submits an activity proof, which has the minimum
        // Hamming distance among all proofs.
        let rewards_tracker = rewards_tracker
            .update_active(
                provider1,
                &ActivityMetadata::Blend(blend::ActivityProof {
                    session: 0,
                    proof_of_quota: new_proof_of_quota_unchecked(1),
                    proof_of_selection: new_proof_of_selection_unchecked(1),
                }),
                config.session_duration,
            )
            .unwrap();

        // provider2 submits an activity proof.
        let rewards_tracker = rewards_tracker
            .update_active(
                provider2,
                &ActivityMetadata::Blend(blend::ActivityProof {
                    session: 0,
                    proof_of_quota: new_proof_of_quota_unchecked(2),
                    proof_of_selection: new_proof_of_selection_unchecked(2),
                }),
                config.session_duration,
            )
            .unwrap();

        // provider3 submits an activity proof, which has the minimum
        // Hamming distance among all proofs.
        let rewards_tracker = rewards_tracker
            .update_active(
                provider3,
                // Use the same proof as provider1 just for testing
                &ActivityMetadata::Blend(blend::ActivityProof {
                    session: 0,
                    proof_of_quota: new_proof_of_quota_unchecked(1),
                    proof_of_selection: new_proof_of_selection_unchecked(1),
                }),
                config.session_duration,
            )
            .unwrap();

        // provider4 doesn't submit an activity proof.

        // Update session from 1 to 2.
        let (_, reward_utxos) = rewards_tracker.update_session(
            &create_test_session_state(
                &[provider1, provider2, provider3, provider4],
                ServiceType::BlendNetwork,
                1,
            ),
            &ZkHash::ZERO,
            &config,
        );

        assert_eq!(reward_utxos.len(), 3); // except provider4

        let Rewards::Initialized { declarations, .. } = rewards_tracker else {
            panic!("rewards_tracker should be in Initialized state");
        };
        let zk_id_to_provider_id: HashMap<PublicKey, ProviderId> = declarations
            .iter()
            .map(|(provider_id, zk_id)| (*zk_id, *provider_id))
            .collect();
        let rewards: HashMap<ProviderId, u64> = reward_utxos
            .iter()
            .map(|utxo| {
                let provider_id = zk_id_to_provider_id
                    .get(&utxo.note.pk)
                    .expect("provider should exist");
                (*provider_id, utxo.note.value)
            })
            .collect();

        // Provider1 and provider3 should get double rewards compared to provider2.
        assert_eq!(
            *rewards.get(&provider1).unwrap(),
            rewards.get(&provider2).unwrap() * 2
        );
        assert_eq!(
            *rewards.get(&provider3).unwrap(),
            rewards.get(&provider2).unwrap() * 2
        );
        // Provider4 should get no rewards.
        assert_eq!(rewards.get(&provider4), None);
    }

    #[test]
    fn test_blend_duplicate_active_messages() {
        let provider1 = create_provider_id(1);

        // Create a reward tracker, and update session from 0 to 1.
        let config = create_service_parameters();
        let (rewards_tracker, _) = Rewards::new(create_blend_rewards_params(864_000, 1))
            .update_session(
                &create_test_session_state(&[provider1], ServiceType::BlendNetwork, 0),
                &ZkHash::ZERO,
                &config,
            );

        // provider1 submits an activity proof.
        let rewards_tracker = rewards_tracker
            .update_active(
                provider1,
                &ActivityMetadata::Blend(blend::ActivityProof {
                    session: 0,
                    proof_of_quota: new_proof_of_quota_unchecked(1),
                    proof_of_selection: new_proof_of_selection_unchecked(1),
                }),
                config.session_duration,
            )
            .unwrap();

        // provider1 submits another activity proof in the same session,
        // which should error.
        let err = rewards_tracker
            .update_active(
                provider1,
                &ActivityMetadata::Blend(blend::ActivityProof {
                    session: 0,
                    proof_of_quota: new_proof_of_quota_unchecked(2),
                    proof_of_selection: new_proof_of_selection_unchecked(2),
                }),
                config.session_duration,
            )
            .unwrap_err();
        assert_eq!(
            err,
            Error::DuplicateActiveMessage {
                session: 0,
                provider_id: Box::new(provider1)
            }
        );
    }

    #[test]
    fn test_blend_invalid_session() {
        let provider1 = create_provider_id(1);

        // Create a reward tracker, and update session from 0 to 1.
        let config = create_service_parameters();
        let (rewards_tracker, _) = Rewards::new(create_blend_rewards_params(864_000, 1))
            .update_session(
                &create_test_session_state(&[provider1], ServiceType::BlendNetwork, 0),
                &ZkHash::ZERO,
                &config,
            );

        // provider1 submits an activity proof with invalid session.
        let err = rewards_tracker
            .update_active(
                provider1,
                &ActivityMetadata::Blend(blend::ActivityProof {
                    session: 99,
                    proof_of_quota: new_proof_of_quota_unchecked(1),
                    proof_of_selection: new_proof_of_selection_unchecked(1),
                }),
                config.session_duration,
            )
            .unwrap_err();
        assert_eq!(
            err,
            Error::InvalidSession {
                expected: 0,
                got: 99
            }
        );

        // No reward should be calculated after session 1.
        let (_, rewards) = rewards_tracker.update_session(
            &create_test_session_state(&[provider1], ServiceType::BlendNetwork, 1),
            &ZkHash::ZERO,
            &config,
        );
        assert_eq!(rewards.len(), 0);
    }

    #[test]
    fn test_blend_network_too_small() {
        let provider1 = create_provider_id(1);

        // Create a reward tracker, and update session from 0 to 1.
        let config = create_service_parameters();
        let (rewards_tracker, _) = Rewards::new(
            // Set minimum network size to 2
            create_blend_rewards_params(864_000, 2),
        )
        .update_session(
            &create_test_session_state(&[provider1], ServiceType::BlendNetwork, 0),
            &ZkHash::ZERO,
            &config,
        );

        // provider1 submits an activity proof, but it should be rejected
        // since the network is too small.
        let err = rewards_tracker
            .update_active(
                provider1,
                &ActivityMetadata::Blend(blend::ActivityProof {
                    session: 0,
                    proof_of_quota: new_proof_of_quota_unchecked(1),
                    proof_of_selection: new_proof_of_selection_unchecked(1),
                }),
                config.session_duration,
            )
            .unwrap_err();
        assert_eq!(
            err,
            Error::MinimumNetworkSizeNotSatisfied {
                num_declarations: 1,
                minimum_network_size: NonZeroU64::new(2).unwrap()
            }
        );

        // No reward should be calculated after session 1.
        let (_, rewards) = rewards_tracker.update_session(
            &create_test_session_state(&[provider1], ServiceType::BlendNetwork, 1),
            &ZkHash::ZERO,
            &config,
        );
        assert_eq!(rewards.len(), 0);
    }

    #[test]
    fn test_blend_proof_distance_larger_than_activity_threshold() {
        let provider1 = create_provider_id(1);

        // Create a reward tracker, and update session from 0 to 1.
        let config = create_service_parameters();
        let (rewards_tracker, _) = Rewards::new(create_blend_rewards_params(10, 1)).update_session(
            &create_test_session_state(&[provider1], ServiceType::BlendNetwork, 0),
            &ZkHash::from(BigUint::from(9999u64)),
            &config,
        );

        // provider1 submits an activity proof that is larger than activity threshold.
        let err = rewards_tracker
            .update_active(
                provider1,
                &ActivityMetadata::Blend(blend::ActivityProof {
                    session: 0,
                    proof_of_quota: new_proof_of_quota_unchecked(2),
                    proof_of_selection: new_proof_of_selection_unchecked(2),
                }),
                config.session_duration,
            )
            .unwrap_err();
        assert_eq!(err, Error::InvalidProof);

        // No reward should be calculated after session 1.
        let (_, rewards) = rewards_tracker.update_session(
            &create_test_session_state(&[provider1], ServiceType::BlendNetwork, 1),
            &ZkHash::ZERO,
            &config,
        );
        assert_eq!(rewards.len(), 0);
    }
}
