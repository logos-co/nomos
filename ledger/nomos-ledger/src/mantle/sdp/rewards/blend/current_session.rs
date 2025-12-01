use cryptarchia_engine::Epoch;
use nomos_blend_message::{
    crypto::proofs::PoQVerificationInputsMinusSigningKey,
    encap::ProofsVerifier as ProofsVerifierTrait, reward::SessionRandomness,
};
use nomos_blend_proofs::{
    merkle::MerkleTree,
    quota::inputs::prove::public::{CoreInputs, LeaderInputs},
};
use nomos_core::{
    crypto::ZkHash,
    sdp::{ProviderId, SessionNumber},
};
use rpds::HashTrieMapSync;
use zksign::PublicKey;

use crate::{
    EpochState,
    mantle::sdp::{
        SessionState,
        rewards::blend::{RewardsParameters, target_session::TargetSessionState},
    },
};

/// Immutable state of the current session.
/// The current session is `s` if `s-1` is the target session for which rewards
/// are being calculated.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CurrentSessionState {
    /// Current session randomness
    session_randomness: SessionRandomness,
}

impl CurrentSessionState {
    const fn new(session_randomness: SessionRandomness) -> Self {
        Self { session_randomness }
    }

    pub const fn session_randomness(&self) -> SessionRandomness {
        self.session_randomness
    }
}

/// Collects epoch states seen in the current session.
/// The current session is `s` if `s-1` is the target session for which rewards
/// are being calculated.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CurrentSessionTracker {
    /// The latest epoch seen in the current session.
    latest_epoch: Epoch,
    /// Collecting leader inputs derived from epoch states seen in the current
    /// session. These will be used to create proof verifiers after the next
    /// session update.
    leader_inputs: Vec<LeaderInputs>,
}

impl CurrentSessionTracker {
    pub fn new(first_epoch_state: &EpochState, settings: &RewardsParameters) -> Self {
        Self {
            latest_epoch: first_epoch_state.epoch,
            leader_inputs: vec![settings.leader_inputs(first_epoch_state)],
        }
    }

    pub fn collect_epoch(&self, epoch_state: &EpochState, settings: &RewardsParameters) -> Self {
        if epoch_state.epoch > self.latest_epoch {
            let mut leader_inputs = self.leader_inputs.clone();
            leader_inputs.push(settings.leader_inputs(epoch_state));
            Self {
                latest_epoch: epoch_state.epoch,
                leader_inputs,
            }
        } else {
            self.clone()
        }
    }

    pub fn finalize<ProofsVerifier>(
        &self,
        last_active_session_state: &SessionState,
        next_session_first_epoch_state: &EpochState,
        settings: &RewardsParameters,
    ) -> CurrentSessionTrackerFinalization<ProofsVerifier>
    where
        ProofsVerifier: ProofsVerifierTrait,
    {
        if last_active_session_state.declarations.size()
            < settings.minimum_network_size.get() as usize
        {
            return CurrentSessionTrackerFinalization::WithoutTargetSession(Self::new(
                next_session_first_epoch_state,
                settings,
            ));
        }

        let (providers, zk_root) = Self::providers_and_zk_root(last_active_session_state);

        let (core_quota, token_evaluation) = settings.core_quota_and_token_evaluation(
            providers.size() as u64,
        ).expect("evaluation parameters shouldn't overflow. panicking since we can't process the new session");

        let proof_verifiers = Self::create_proof_verifiers(
            &self.leader_inputs,
            last_active_session_state.session_n,
            zk_root,
            core_quota,
        );

        CurrentSessionTrackerFinalization::WithTargetSession {
            target_session_state: TargetSessionState::new(
                last_active_session_state.session_n,
                providers,
                token_evaluation,
                proof_verifiers,
            ),
            current_session_state: CurrentSessionState::new(SessionRandomness::new(
                last_active_session_state.session_n + 1,
                &next_session_first_epoch_state.nonce,
            )),
            current_session_tracker: Self::new(next_session_first_epoch_state, settings),
        }
    }

    fn providers_and_zk_root(
        session_state: &SessionState,
    ) -> (HashTrieMapSync<ProviderId, (PublicKey, u64)>, ZkHash) {
        let mut providers = session_state
            .declarations
            .values()
            .map(|declaration| (declaration.provider_id, declaration.zk_id))
            .collect::<Vec<_>>();
        providers.sort_by_key(|(_, zk_id)| *zk_id);

        let (providers, zk_ids) = providers.into_iter().enumerate().fold(
            (HashTrieMapSync::new_sync(), Vec::new()),
            |(mut provider_indices, mut zk_ids), (i, (provider_id, zk_id))| {
                provider_indices.insert_mut(
                    provider_id,
                    (
                        zk_id,
                        u64::try_from(i).expect("provider index must fit in u64"),
                    ),
                );
                zk_ids.push(zk_id);
                (provider_indices, zk_ids)
            },
        );

        let zk_root = MerkleTree::new_from_ordered(zk_ids)
            .expect("Should not fail to build merkle tree of core nodes' zk public keys")
            .root();

        (providers, zk_root)
    }

    fn create_proof_verifiers<ProofsVerifier: ProofsVerifierTrait>(
        leader_inputs: &[LeaderInputs],
        session: SessionNumber,
        zk_root: ZkHash,
        core_quota: u64,
    ) -> Vec<ProofsVerifier> {
        leader_inputs
            .iter()
            .map(|&leader| {
                ProofsVerifier::new(PoQVerificationInputsMinusSigningKey {
                    session,
                    core: CoreInputs {
                        zk_root,
                        quota: core_quota,
                    },
                    leader,
                })
            })
            .collect()
    }

    #[cfg(test)]
    pub const fn num_leader_inputs(&self) -> usize {
        self.leader_inputs.len()
    }
}

pub enum CurrentSessionTrackerFinalization<ProofsVerifier> {
    WithTargetSession {
        target_session_state: TargetSessionState<ProofsVerifier>,
        current_session_state: CurrentSessionState,
        current_session_tracker: CurrentSessionTracker,
    },
    WithoutTargetSession(CurrentSessionTracker),
}
