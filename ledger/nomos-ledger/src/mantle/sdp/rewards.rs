use std::{
    cmp::min,
    collections::{HashMap, HashSet},
    num::NonZeroU64,
};

use groth16::Fr;
use nomos_blend_message::reward::{BlendingTokenEvaluation, SessionRandomness};
use nomos_core::{
    blend::core_quota,
    block::BlockNumber,
    sdp::{ActivityMetadata, ProviderId, ServiceParameters, SessionNumber},
};
use nomos_utils::math::NonNegativeF64;
use rpds::{HashTrieMapSync, HashTrieSetSync};
use thiserror::Error;

use super::SessionState;

pub type RewardAmount = u64;
const ACTIVITY_THRESHOLD: u64 = 2;

/// Generic trait for service-specific reward calculation.
///
/// Each service can implement its own rewards logic by implementing this trait.
/// The rewards object is updated with active messages and session transitions,
/// and can calculate expected rewards for each provider based on the service's
/// internal logic.
pub trait Rewards: Clone + PartialEq + Send + Sync + std::fmt::Debug {
    /// Update rewards state when an active message is received.
    ///
    /// Called when a provider submits an active message with metadata
    /// (e.g., activity proofs containing opinions about other providers).
    fn update_active(
        &self,
        declaration_id: ProviderId,
        metadata: &ActivityMetadata,
        block_number: BlockNumber,
    ) -> Result<Self, Error>;

    /// Update rewards state when sessions transition and calculate rewards to
    /// distribute.
    ///
    /// Called during session boundaries when active, `past_session`, and
    /// forming sessions are updated. Returns a map of `ProviderId` to
    /// reward amounts for providers eligible for rewards in this session
    /// transition.
    ///
    /// The internal calculation logic is opaque to the SDP ledger and
    /// determined by the service-specific implementation.
    ///
    /// # Arguments
    /// * `last_active` - The state of the session that just ended.
    /// * `next_active_session_epoch_nonce` - The nonce of the epoch state
    ///   corresponding to the 1st block of the session `last_active + 1`.
    fn update_session(
        &self,
        last_active: &SessionState,
        next_active_session_epoch_nonce: &Fr,
        config: &ServiceParameters,
    ) -> (Self, HashMap<ProviderId, RewardAmount>);
}

/// Data Availability rewards implementation based on opinion-based peer
/// evaluation.
///
/// Implements the `NomosDA` Rewarding specification where providers submit
/// activity proofs containing opinions about peer service quality, and rewards
/// are distributed based on accumulated positive opinions exceeding a
/// threshold.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DaRewards {
    current_opinions: HashTrieMapSync<ProviderId, usize>,
    past_opinions: HashTrieMapSync<ProviderId, usize>,
    recorded_messages: HashTrieSetSync<ProviderId>, // avoid processing duplicate opinions
    // naming as in the spec, current session is s-1 if s is the session at this which this
    // message was sent
    // current rewarding session s - 1  (s is active session)
    current_session: SessionState,
    // previous rewarding session s - 2 (s is active session)
    prev_session: SessionState,
}

impl Default for DaRewards {
    fn default() -> Self {
        Self::new() // Default to 2 (same as previous ACTIVITY_THRESHOLD constant)
    }
}

impl DaRewards {
    /// Create a new `DaRewards` instance with the specified opinion threshold
    #[must_use]
    pub fn new() -> Self {
        Self {
            current_opinions: HashTrieMapSync::new_sync(),
            past_opinions: HashTrieMapSync::new_sync(),
            recorded_messages: HashTrieSetSync::new_sync(),
            current_session: SessionState {
                declarations: rpds::RedBlackTreeMapSync::new_sync(),
                session_n: 0,
            },
            prev_session: SessionState {
                declarations: rpds::RedBlackTreeMapSync::new_sync(),
                session_n: 0,
            },
        }
    }

    fn parse_opinions(opinions: &[u8], n_validators: usize) -> Result<Vec<bool>, Error> {
        let expected_current_len = Self::calculate_opinion_vector_length(n_validators);
        if opinions.len() != expected_current_len {
            return Err(Error::InvalidOpinionLength {
                expected: expected_current_len,
                got: opinions.len(),
            });
        }

        // * self opinion is not checked
        // * we only check opinions up to the number of validators, without checking the
        //   rest of the opinion bits are zero

        Ok((0..n_validators)
            .map(|i| Self::get_opinion_bit(opinions, i))
            .collect())
    }

    /// Calculate expected byte length for opinion vector: ⌈log₂(Ns + 1) / 8⌉
    const fn calculate_opinion_vector_length(node_count: usize) -> usize {
        let bits_needed = (node_count + 1).next_power_of_two().trailing_zeros() as usize;
        bits_needed.div_ceil(8)
    }

    /// Get opinion bit at index i (little-endian encoding)
    fn get_opinion_bit(opinions: &[u8], index: usize) -> bool {
        let byte_index = index / 8;
        let bit_index = index % 8;
        if byte_index >= opinions.len() {
            return false;
        }
        (opinions[byte_index] & (1 << bit_index)) != 0
    }
}

impl Rewards for DaRewards {
    fn update_active(
        &self,
        provider_id: ProviderId,
        metadata: &ActivityMetadata,
        _block_number: BlockNumber,
    ) -> Result<Self, Error> {
        // Extract DA activity proof from metadata
        let ActivityMetadata::DataAvailability(proof) = metadata else {
            return Err(Error::InvalidProofType);
        };

        if self.recorded_messages.contains(&provider_id) {
            return Err(Error::DuplicateActiveMessage {
                session: proof.current_session,
                provider_id: Box::new(provider_id),
            });
        }

        if proof.current_session != self.current_session.session_n {
            return Err(Error::InvalidSession {
                expected: self.current_session.session_n,
                got: proof.current_session,
            });
        }

        // Process current session opinions
        let n_validators_current = self.current_session.declarations.size();
        let current_opinions =
            Self::parse_opinions(&proof.current_session_opinions, n_validators_current)?;

        let mut new_current_opinions = self.current_opinions.clone();
        for (i, &opinion) in current_opinions.iter().enumerate() {
            if opinion && let Some(provider) = self.current_session.get_provider_by_index(i) {
                let count = new_current_opinions.get(&provider).copied().unwrap_or(0);
                new_current_opinions = new_current_opinions.insert(provider, count + 1);
            }
        }

        // Process previous session opinions
        let n_validators_past = self.prev_session.declarations.size();
        let past_opinions =
            Self::parse_opinions(&proof.previous_session_opinions, n_validators_past)?;

        let mut new_past_opinions = self.past_opinions.clone();
        for (i, &opinion) in past_opinions.iter().enumerate() {
            if opinion && let Some(provider) = self.prev_session.get_provider_by_index(i) {
                let count = new_past_opinions.get(&provider).copied().unwrap_or(0);
                new_past_opinions = new_past_opinions.insert(provider, count + 1);
            }
        }

        let new_recorded_messages = self.recorded_messages.insert(provider_id);

        Ok(Self {
            current_opinions: new_current_opinions,
            past_opinions: new_past_opinions,
            recorded_messages: new_recorded_messages,
            current_session: self.current_session.clone(),
            prev_session: self.prev_session.clone(),
        })
    }

    fn update_session(
        &self,
        last_active: &SessionState,
        _next_active_session_epoch_nonce: &Fr,
        _config: &ServiceParameters,
    ) -> (Self, HashMap<ProviderId, RewardAmount>) {
        // Calculate activity threshold: θ = Ns / ACTIVITY_THRESHOLD
        let active_threshold = self.current_session.declarations.size() as u64 / ACTIVITY_THRESHOLD;
        let past_threshold = self.prev_session.declarations.size() as u64 / ACTIVITY_THRESHOLD;

        // TODO: Calculate base rewards when session_income is added to config
        // For now using placeholder value of 0
        let session_income = 0;

        // Calculate base rewards
        let active_base_reward = if self.current_session.declarations.is_empty() {
            0
        } else {
            session_income / self.current_session.declarations.size() as u64
        };

        let past_base_reward = if self.prev_session.declarations.is_empty() {
            0
        } else {
            session_income / self.prev_session.declarations.size() as u64
        };

        let mut rewards = HashMap::new();
        // Distribute rewards for current session
        for (provider_id, &opinion_count) in self.current_opinions.iter() {
            if (opinion_count as u64) >= active_threshold {
                let reward = active_base_reward / 2; // half reward
                *rewards.entry(*provider_id).or_insert(0) += reward;
            }
        }

        // Distribute rewards for previous session
        for (provider_id, &opinion_count) in self.past_opinions.iter() {
            if (opinion_count as u64) >= past_threshold {
                let reward = past_base_reward / 2; // half reward
                *rewards.entry(*provider_id).or_insert(0) += reward;
            }
        }

        // Create new rewards state with updated sessions
        let new_state = Self {
            current_opinions: HashTrieMapSync::new_sync(), // Reset for new session
            past_opinions: HashTrieMapSync::new_sync(),    // Move current to past
            recorded_messages: HashTrieSetSync::new_sync(), // Reset recorded messages
            current_session: last_active.clone(),
            prev_session: self.current_session.clone(),
        };

        (new_state, rewards)
    }
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq)]
pub enum BlendRewards {
    /// State before the first session update (0 -> 1).
    /// No activity messages are accepted in this state, because activity cannot
    /// exist before session 0.
    Uninitialized(BlendRewardsParameters),
    /// State after the first session update (0 -> 1).
    /// This is updated every new session.
    Initialized {
        /// State of the session that is referenced by the submitted proofs.
        /// This is `s-1` if `s` is the session at which this message was sent.
        session_state: SessionState,
        /// Parameters for evaluating activity proofs in the session
        token_evaluation: BlendingTokenEvaluation,
        /// Session randomness for the session `s`.
        next_session_randomness: SessionRandomness,
        /// Proofs submitted by providers in the session corresponding to
        /// `session_state`.
        submitted_proofs: HashTrieMapSync<ProviderId, u64>,
        /// Tracking the minimum Hamming distance among submitted proofs.
        min_hamming_distance: u64,
        /// Settings that don't change per session.
        settings: BlendRewardsParameters,
    },
}

impl BlendRewards {
    /// Create a new uninitialized [`BlendRewards`] that doesn't accept activity
    /// messages until the first session update.
    #[must_use]
    pub const fn new(settings: BlendRewardsParameters) -> Self {
        Self::Uninitialized(settings)
    }
}

impl Rewards for BlendRewards {
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
                session_state,
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

                if proof.session != session_state.session_n {
                    return Err(Error::InvalidSession {
                        expected: session_state.session_n,
                        got: proof.session,
                    });
                }

                let num_declarations = session_state.declarations.size() as u64;
                if num_declarations < settings.minimum_network_size.get() {
                    return Err(Error::MinimumNetworkSizeNotSatisfied {
                        num_declarations,
                        minimum_network_size: settings.minimum_network_size,
                    });
                }

                let proof = nomos_blend_message::reward::ActivityProof::try_from(proof)
                    .map_err(|_| Error::InvalidProofType)?;

                // TODO: Validate PoQ and PoSel by holding all epoch infos that
                // spanned across the `session_state.session_n` session.

                let Some(hamming_distance) =
                    token_evaluation.evaluate(proof.token(), *next_session_randomness)
                else {
                    return Err(Error::InvalidProof);
                };

                Ok(Self::Initialized {
                    session_state: session_state.clone(),
                    token_evaluation: *token_evaluation,
                    next_session_randomness: *next_session_randomness,
                    submitted_proofs: submitted_proofs.insert(provider_id, hamming_distance),
                    min_hamming_distance: min(*min_hamming_distance, hamming_distance),
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
    ) -> (Self, HashMap<ProviderId, RewardAmount>) {
        match self {
            Self::Uninitialized(settings) => {
                // Prepare the proof evaluation parameters for the new session
                let token_evaluation = settings.token_evaluation(
                    last_active.declarations.size() as u64,
                ).expect("evaluation parameters shouldn't overflow. panicking since we can't process the new session");

                (
                    Self::Initialized {
                        session_state: last_active.clone(),
                        token_evaluation,
                        next_session_randomness: SessionRandomness::new(
                            last_active.session_n + 1,
                            next_active_session_epoch_nonce,
                        ),
                        submitted_proofs: HashTrieMapSync::new_sync(),
                        min_hamming_distance: u64::MAX,
                        settings: settings.clone(),
                    },
                    HashMap::new(),
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
                let premium_providers = submitted_proofs
                    .iter()
                    .filter_map(|(&id, &hamming_distance)| {
                        (hamming_distance == *min_hamming_distance).then_some(id)
                    })
                    .collect::<HashSet<_>>();

                // Calculate base reward
                let base_reward = if submitted_proofs.is_empty() {
                    0
                } else {
                    session_income
                        / (submitted_proofs.size() as u64 + premium_providers.len() as u64)
                };

                // Calculate reward for each provider
                let mut rewards = HashMap::new();
                for provider_id in submitted_proofs.keys() {
                    let reward = if premium_providers.contains(provider_id) {
                        base_reward * 2
                    } else {
                        base_reward
                    };
                    rewards.insert(*provider_id, reward);
                }

                // Prepare the proof evaluation parameters for the new session
                let token_evaluation = settings.token_evaluation(
                    last_active.declarations.size() as u64,
                ).expect("evaluation parameters shouldn't overflow. panicking since we can't process the new session");

                // Create new rewards state with updated sessions
                let new_state = Self::Initialized {
                    session_state: last_active.clone(),
                    token_evaluation,
                    next_session_randomness: SessionRandomness::new(
                        last_active.session_n + 1,
                        next_active_session_epoch_nonce,
                    ),
                    submitted_proofs: HashTrieMapSync::new_sync(),
                    min_hamming_distance: u64::MAX,
                    settings: settings.clone(),
                };

                (new_state, rewards)
            }
        }
    }
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct BlendRewardsParameters {
    pub rounds_per_session: NonZeroU64,
    pub message_frequency_per_round: NonNegativeF64,
    pub num_blend_layers: NonZeroU64,
    pub minimum_network_size: NonZeroU64,
}

impl BlendRewardsParameters {
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

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum Error {
    #[error("Rewards state is not initialized yet with a real session")]
    Uninitialized,
    #[error("Invalid session: expected {expected}, got {got}")]
    InvalidSession {
        expected: SessionNumber,
        got: SessionNumber,
    },
    #[error("Invalid opinion length: expected {expected}, got {got}")]
    InvalidOpinionLength { expected: usize, got: usize },
    #[error("Duplicate active message for session {session}, provider {provider_id:?}")]
    DuplicateActiveMessage {
        session: SessionNumber,
        provider_id: Box<ProviderId>,
    },
    #[error("Invalid proof type")]
    InvalidProofType,
    #[error("Invalid proof")]
    InvalidProof,
    #[error(
        "The number of declarations ({num_declarations}) is less than the minimum network size ({minimum_network_size})"
    )]
    MinimumNetworkSizeNotSatisfied {
        num_declarations: u64,
        minimum_network_size: NonZeroU64,
    },
}

#[cfg(test)]
mod tests {
    use groth16::Field as _;
    use nomos_core::{
        crypto::ZkHash,
        sdp::{BlendActivityProof, DaActivityProof, Declaration, DeclarationId, ServiceType},
    };
    use num_bigint::BigUint;
    use zksign::PublicKey;

    use super::*;

    fn create_test_session_state(
        provider_ids: &[ProviderId],
        service_type: ServiceType,
        session_n: SessionNumber,
    ) -> SessionState {
        let mut declarations = rpds::RedBlackTreeMapSync::new_sync();
        for (i, provider_id) in provider_ids.iter().enumerate() {
            let declaration = Declaration {
                service_type,
                provider_id: *provider_id,
                locked_note_id: Fr::from(i as u64).into(),
                locators: vec![],
                zk_id: PublicKey::new(BigUint::from(i as u64).into()),
                created: 0,
                active: 0,
                withdrawn: None,
                nonce: 0,
            };
            declarations = declarations.insert(DeclarationId([i as u8; 32]), declaration);
        }
        SessionState {
            declarations,
            session_n,
        }
    }

    fn create_provider_id(byte: u8) -> ProviderId {
        use ed25519_dalek::SigningKey;
        let key_bytes = [byte; 32];
        // Ensure the key is valid by using SigningKey
        let signing_key = SigningKey::from_bytes(&key_bytes);
        ProviderId(signing_key.verifying_key())
    }

    fn create_service_parameters() -> ServiceParameters {
        ServiceParameters {
            lock_period: 10,
            inactivity_period: 20,
            retention_period: 100,
            timestamp: 0,
            session_duration: 10,
        }
    }

    fn create_blend_rewards_params(minimum_network_size: u64) -> BlendRewardsParameters {
        BlendRewardsParameters {
            rounds_per_session: NonZeroU64::new(100).unwrap(),
            message_frequency_per_round: NonNegativeF64::try_from(1.0).unwrap(),
            num_blend_layers: NonZeroU64::new(3).unwrap(),
            minimum_network_size: minimum_network_size.try_into().unwrap(),
        }
    }

    #[test]
    fn test_calculate_opinion_vector_length() {
        // 0 nodes: 0 bytes
        assert_eq!(DaRewards::calculate_opinion_vector_length(0), 0);

        // 1-2 nodes: need 2 bits -> 1 byte
        assert_eq!(DaRewards::calculate_opinion_vector_length(1), 1);
        assert_eq!(DaRewards::calculate_opinion_vector_length(2), 1);

        // 3-4 nodes: need 3 bits -> 1 byte
        assert_eq!(DaRewards::calculate_opinion_vector_length(3), 1);
        assert_eq!(DaRewards::calculate_opinion_vector_length(4), 1);

        // 5-8 nodes: need 4 bits -> 1 byte
        assert_eq!(DaRewards::calculate_opinion_vector_length(8), 1);

        // 9-16 nodes: need 5 bits -> 1 byte
        assert_eq!(DaRewards::calculate_opinion_vector_length(16), 1);

        // 17-32 nodes: need 6 bits -> 1 byte
        assert_eq!(DaRewards::calculate_opinion_vector_length(32), 1);

        // 33-64 nodes: need 7 bits -> 1 byte
        assert_eq!(DaRewards::calculate_opinion_vector_length(64), 1);

        // 65-128 nodes: need 8 bits -> 1 byte
        assert_eq!(DaRewards::calculate_opinion_vector_length(128), 1);

        // 129-256 nodes: need 9 bits -> 2 bytes
        assert_eq!(DaRewards::calculate_opinion_vector_length(256), 2);
    }

    #[test]
    fn test_get_opinion_bit() {
        let opinions = vec![0b1011_0100, 0b0000_0011];

        // First byte: bits 0-7
        assert!(!DaRewards::get_opinion_bit(&opinions, 0)); // 0
        assert!(!DaRewards::get_opinion_bit(&opinions, 1)); // 0
        assert!(DaRewards::get_opinion_bit(&opinions, 2)); // 1
        assert!(!DaRewards::get_opinion_bit(&opinions, 3)); // 0
        assert!(DaRewards::get_opinion_bit(&opinions, 4)); // 1
        assert!(DaRewards::get_opinion_bit(&opinions, 5)); // 1
        assert!(!DaRewards::get_opinion_bit(&opinions, 6)); // 0
        assert!(DaRewards::get_opinion_bit(&opinions, 7)); // 1

        // Second byte: bits 8-15
        assert!(DaRewards::get_opinion_bit(&opinions, 8)); // 1
        assert!(DaRewards::get_opinion_bit(&opinions, 9)); // 1
        assert!(!DaRewards::get_opinion_bit(&opinions, 10)); // 0

        // Out of bounds
        assert!(!DaRewards::get_opinion_bit(&opinions, 100));
    }

    #[test]
    fn test_get_provider_by_index() {
        let provider1 = create_provider_id(1);
        let provider2 = create_provider_id(2);
        let provider3 = create_provider_id(3);

        let session = create_test_session_state(
            &[provider2, provider1, provider3],
            ServiceType::DataAvailability,
            0,
        );

        // Test that we can get providers by index
        let p0 = session.get_provider_by_index(0);
        let p1 = session.get_provider_by_index(1);
        let p2 = session.get_provider_by_index(2);

        // All three should exist
        assert!(p0.is_some());
        assert!(p1.is_some());
        assert!(p2.is_some());

        // They should all be different
        assert_ne!(p0, p1);
        assert_ne!(p1, p2);
        assert_ne!(p0, p2);

        // Out of bounds should return None
        assert!(session.get_provider_by_index(3).is_none());
    }

    #[test]
    fn test_da_rewards_with_no_activity_proofs() {
        let provider1 = create_provider_id(1);
        let provider2 = create_provider_id(2);

        // Create active session with providers
        let active_session =
            create_test_session_state(&[provider1, provider2], ServiceType::DataAvailability, 1);

        // Initialize rewards tracker with the active session
        let rewards_tracker = DaRewards {
            current_opinions: HashTrieMapSync::new_sync(),
            past_opinions: HashTrieMapSync::new_sync(),
            recorded_messages: HashTrieSetSync::new_sync(),
            current_session: active_session.clone(),
            prev_session: SessionState {
                declarations: rpds::RedBlackTreeMapSync::new_sync(),
                session_n: 0,
            },
        };

        let config = ServiceParameters {
            lock_period: 10,
            inactivity_period: 20,
            retention_period: 100,
            timestamp: 0,
            session_duration: 10,
        };

        let (_new_state, rewards) =
            rewards_tracker.update_session(&active_session, &ZkHash::ZERO, &config);

        // No activity proofs submitted, so no rewards
        assert_eq!(rewards.len(), 0);
    }

    #[test]
    fn test_da_rewards_basic_calculation() {
        // Create 4 providers
        let provider1 = create_provider_id(1);
        let provider2 = create_provider_id(2);
        let provider3 = create_provider_id(3);
        let provider4 = create_provider_id(4);

        let providers = vec![provider1, provider2, provider3, provider4];
        let active_session =
            create_test_session_state(&providers, ServiceType::DataAvailability, 1);

        // Initialize rewards tracker with the active session
        let mut rewards_tracker = DaRewards {
            current_opinions: HashTrieMapSync::new_sync(),
            past_opinions: HashTrieMapSync::new_sync(),
            recorded_messages: HashTrieSetSync::new_sync(),
            current_session: active_session.clone(),
            prev_session: SessionState {
                declarations: rpds::RedBlackTreeMapSync::new_sync(),
                session_n: 0,
            },
        };

        // Helper to create opinion vector with all positive opinions
        let create_all_positive = || vec![0b0000_1111u8]; // All 4 providers positive

        // Provider 1 submits: positive about all
        let proof1 = DaActivityProof {
            current_session: 1,
            previous_session_opinions: vec![],
            current_session_opinions: create_all_positive(),
        };
        rewards_tracker = rewards_tracker
            .update_active(provider1, &ActivityMetadata::DataAvailability(proof1), 10)
            .unwrap();

        // Provider 2 submits: positive about first 3 (bits 0, 1, 2)
        let proof2 = DaActivityProof {
            current_session: 1,
            previous_session_opinions: vec![],
            current_session_opinions: vec![0b0000_0111u8],
        };
        rewards_tracker = rewards_tracker
            .update_active(provider2, &ActivityMetadata::DataAvailability(proof2), 10)
            .unwrap();

        // Provider 3 submits: positive about all
        let proof3 = DaActivityProof {
            current_session: 1,
            previous_session_opinions: vec![],
            current_session_opinions: create_all_positive(),
        };
        rewards_tracker = rewards_tracker
            .update_active(provider3, &ActivityMetadata::DataAvailability(proof3), 10)
            .unwrap();

        let config = ServiceParameters {
            lock_period: 10,
            inactivity_period: 20,
            retention_period: 100,
            timestamp: 0,
            session_duration: 10,
        };

        let (_new_state, rewards) =
            rewards_tracker.update_session(&active_session, &ZkHash::ZERO, &config);

        // Activity threshold = 4 / 2 = 2
        // NOTE: session_income is currently hardcoded to 0, so all rewards will be 0
        // When session_income is added to config, this test should verify:
        // Base reward = session_income / 4
        // Half reward = base_reward / 2
        // All providers should get rewards as they all have >= 2 opinions

        // For now, verify that all rewards are 0 (since session_income = 0)
        assert_eq!(rewards.len(), 4);
        assert!(rewards.values().all(|&r| r == 0));
        assert_eq!(rewards.values().sum::<u64>(), 0);
    }

    #[test]
    fn test_da_rewards_with_previous_session() {
        let provider1 = create_provider_id(1);
        let provider2 = create_provider_id(2);

        // Set up sessions: provider1 in both, provider2 only in current
        let current_session =
            create_test_session_state(&[provider1, provider2], ServiceType::DataAvailability, 1);
        let prev_session =
            create_test_session_state(&[provider1], ServiceType::DataAvailability, 0);

        // Initialize rewards tracker with both sessions
        let mut rewards_tracker = DaRewards {
            current_opinions: HashTrieMapSync::new_sync(),
            past_opinions: HashTrieMapSync::new_sync(),
            recorded_messages: HashTrieSetSync::new_sync(),
            current_session: current_session.clone(),
            prev_session,
        };

        // Provider 1 submits opinions for current session
        let proof1 = DaActivityProof {
            current_session: 1,
            previous_session_opinions: vec![0b0000_0001u8], // Positive about provider1 in prev
            current_session_opinions: vec![0b0000_0011u8],  // Positive about both in current
        };
        rewards_tracker = rewards_tracker
            .update_active(provider1, &ActivityMetadata::DataAvailability(proof1), 10)
            .unwrap();

        // Provider 2 submits opinions for current session only
        let proof2 = DaActivityProof {
            current_session: 1,
            previous_session_opinions: vec![0b0000_0001u8], // Positive about provider1 in prev
            current_session_opinions: vec![0b0000_0011u8],  // Positive about both in current
        };
        rewards_tracker = rewards_tracker
            .update_active(provider2, &ActivityMetadata::DataAvailability(proof2), 10)
            .unwrap();

        let config = ServiceParameters {
            lock_period: 10,
            inactivity_period: 20,
            retention_period: 100,
            timestamp: 0,
            session_duration: 10,
        };

        let (_new_state, rewards) =
            rewards_tracker.update_session(&current_session, &ZkHash::ZERO, &config);

        // NOTE: session_income is currently hardcoded to 0, so all rewards will be 0
        // When session_income is added to config, this test should verify:
        // - Both providers get rewards for current session (2 opinions >= threshold 1)
        // - Provider 1 also gets rewards for previous session (2 opinions >= threshold
        //   0)
        // - Provider 1 gets more total rewards (from both sessions)

        // For now, verify that both providers are tracked with 0 rewards
        assert!(rewards.contains_key(&provider1));
        assert!(rewards.contains_key(&provider2));
        assert_eq!(*rewards.get(&provider1).unwrap(), 0);
        assert_eq!(*rewards.get(&provider2).unwrap(), 0);
    }

    #[test]
    fn test_blend_no_reward_calculated_after_session_0() {
        // Create a reward tracker
        let rewards_tracker = BlendRewards::new(create_blend_rewards_params(1));

        // Create session_1 with providers
        let session_1 = create_test_session_state(
            &[create_provider_id(1), create_provider_id(2)],
            ServiceType::BlendNetwork,
            1,
        );

        // Update session from 0 to 1
        let (_, rewards) =
            rewards_tracker.update_session(&session_1, &ZkHash::ZERO, &create_service_parameters());

        // No rewards should be calculated after session 0
        assert_eq!(rewards.len(), 0);
    }

    #[test]
    fn test_blend_rewards_with_no_activity_proofs() {
        // Create a reward tracker, and update session from 0 to 1.
        let config = create_service_parameters();
        let (rewards_tracker, _) = BlendRewards::new(BlendRewardsParameters {
            rounds_per_session: NonZeroU64::new(10).unwrap(),
            message_frequency_per_round: NonNegativeF64::try_from(1.0).unwrap(),
            num_blend_layers: NonZeroU64::new(3).unwrap(),
            minimum_network_size: NonZeroU64::new(1).unwrap(),
        })
        .update_session(
            &create_test_session_state(
                &[create_provider_id(1), create_provider_id(2)],
                ServiceType::BlendNetwork,
                1,
            ),
            &ZkHash::ZERO,
            &config,
        );

        // Update session from 1 to 2 without any activity proofs submitted.
        let (_, rewards) = rewards_tracker.update_session(
            &create_test_session_state(
                &[create_provider_id(1), create_provider_id(2)],
                ServiceType::BlendNetwork,
                2,
            ),
            &ZkHash::ZERO,
            &config,
        );
        assert_eq!(rewards.len(), 0);
    }

    #[test]
    fn test_blend_rewards_calculation() {
        let provider1 = create_provider_id(1);
        let provider2 = create_provider_id(2);
        let provider3 = create_provider_id(3);
        let provider4 = create_provider_id(4);

        // Create a reward tracker, and update session from 0 to 1.
        let config = create_service_parameters();
        let (rewards_tracker, _) = BlendRewards::new(create_blend_rewards_params(1))
            .update_session(
                &create_test_session_state(
                    &[provider1, provider2, provider3, provider4],
                    ServiceType::BlendNetwork,
                    1,
                ),
                &ZkHash::ZERO,
                &config,
            );

        // provider1 submits an activity proof, which has the minimum
        // Hamming distance among all proofs.
        let rewards_tracker = rewards_tracker
            .update_active(
                provider1,
                &ActivityMetadata::Blend(BlendActivityProof {
                    session: 1,
                    proof_of_quota: [1; _],
                    proof_of_selection: [1; _],
                }),
                config.session_duration,
            )
            .unwrap();

        // provider2 submits an activity proof.
        let rewards_tracker = rewards_tracker
            .update_active(
                provider2,
                &ActivityMetadata::Blend(BlendActivityProof {
                    session: 1,
                    proof_of_quota: [2; _],
                    proof_of_selection: [2; _],
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
                &ActivityMetadata::Blend(BlendActivityProof {
                    session: 1,
                    proof_of_quota: [1; _],
                    proof_of_selection: [1; _],
                }),
                config.session_duration,
            )
            .unwrap();

        // provider4 doesn't submit an activity proof.

        // Update session from 1 to 2.
        let (_, rewards) = rewards_tracker.update_session(
            &create_test_session_state(
                &[provider1, provider2, provider3, provider4],
                ServiceType::BlendNetwork,
                2,
            ),
            &ZkHash::ZERO,
            &config,
        );
        // TODO: session_income is currently hardcoded to 0, so all rewards will be 0
        // When session_incom is implemented, this test should verify:
        // - provider1 and provider3 get double rewards of provider2.
        assert_eq!(rewards.len(), 3); // except provider4
        assert_eq!(rewards.get(&provider1), Some(&0));
        assert_eq!(rewards.get(&provider2), Some(&0));
        assert_eq!(rewards.get(&provider3), Some(&0));
        assert_eq!(rewards.get(&provider4), None);
    }

    #[test]
    fn test_blend_duplicate_active_messages() {
        let provider1 = create_provider_id(1);

        // Create a reward tracker, and update session from 0 to 1.
        let config = create_service_parameters();
        let (rewards_tracker, _) = BlendRewards::new(create_blend_rewards_params(1))
            .update_session(
                &create_test_session_state(&[provider1], ServiceType::BlendNetwork, 1),
                &ZkHash::ZERO,
                &config,
            );

        // provider1 submits an activity proof.
        let rewards_tracker = rewards_tracker
            .update_active(
                provider1,
                &ActivityMetadata::Blend(BlendActivityProof {
                    session: 1,
                    proof_of_quota: [1; _],
                    proof_of_selection: [1; _],
                }),
                config.session_duration,
            )
            .unwrap();

        // provider1 submits another activity proof in the same session,
        // which should error.
        let err = rewards_tracker
            .update_active(
                provider1,
                &ActivityMetadata::Blend(BlendActivityProof {
                    session: 1,
                    proof_of_quota: [2; _],
                    proof_of_selection: [2; _],
                }),
                config.session_duration,
            )
            .unwrap_err();
        assert_eq!(
            err,
            Error::DuplicateActiveMessage {
                session: 1,
                provider_id: Box::new(provider1)
            }
        );
    }

    #[test]
    fn test_blend_invalid_session() {
        let provider1 = create_provider_id(1);

        // Create a reward tracker, and update session from 0 to 1.
        let config = create_service_parameters();
        let (rewards_tracker, _) = BlendRewards::new(create_blend_rewards_params(1))
            .update_session(
                &create_test_session_state(&[provider1], ServiceType::BlendNetwork, 1),
                &ZkHash::ZERO,
                &config,
            );

        // provider1 submits an activity proof with invalid session.
        let err = rewards_tracker
            .update_active(
                provider1,
                &ActivityMetadata::Blend(BlendActivityProof {
                    session: 99,
                    proof_of_quota: [1; _],
                    proof_of_selection: [1; _],
                }),
                config.session_duration,
            )
            .unwrap_err();
        assert_eq!(
            err,
            Error::InvalidSession {
                expected: 1,
                got: 99
            }
        );

        // No reward should be calculated after session 1.
        let (_, rewards) = rewards_tracker.update_session(
            &create_test_session_state(&[provider1], ServiceType::BlendNetwork, 2),
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
        let (rewards_tracker, _) = BlendRewards::new(
            // Set minimum network size to 2
            create_blend_rewards_params(2),
        )
        .update_session(
            &create_test_session_state(&[provider1], ServiceType::BlendNetwork, 1),
            &ZkHash::ZERO,
            &config,
        );

        // provider1 submits an activity proof, but it should be rejected
        // since the network is too small.
        let err = rewards_tracker
            .update_active(
                provider1,
                &ActivityMetadata::Blend(BlendActivityProof {
                    session: 1,
                    proof_of_quota: [1; _],
                    proof_of_selection: [1; _],
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
            &create_test_session_state(&[provider1], ServiceType::BlendNetwork, 2),
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
        let (rewards_tracker, _) = BlendRewards::new(create_blend_rewards_params(1))
            .update_session(
                &create_test_session_state(&[provider1], ServiceType::BlendNetwork, 1),
                &ZkHash::from(BigUint::from(9999u64)),
                &config,
            );

        // provider1 submits an activity proof that is larger than activity threshold.
        let err = rewards_tracker
            .update_active(
                provider1,
                &ActivityMetadata::Blend(BlendActivityProof {
                    session: 1,
                    proof_of_quota: [2; _],
                    proof_of_selection: [2; _],
                }),
                config.session_duration,
            )
            .unwrap_err();
        assert_eq!(err, Error::InvalidProof);

        // No reward should be calculated after session 1.
        let (_, rewards) = rewards_tracker.update_session(
            &create_test_session_state(&[provider1], ServiceType::BlendNetwork, 2),
            &ZkHash::ZERO,
            &config,
        );
        assert_eq!(rewards.len(), 0);
    }
}
