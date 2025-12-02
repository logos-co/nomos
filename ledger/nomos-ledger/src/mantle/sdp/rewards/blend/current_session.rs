use cryptarchia_engine::Epoch;
use nomos_blend_message::reward::SessionRandomness;
use nomos_blend_proofs::quota::inputs::prove::public::LeaderInputs;
use nomos_core::sdp::ProviderId;
use rpds::HashTrieMapSync;
use tracing::debug;
use zksign::PublicKey;

use crate::{
    EpochState,
    mantle::sdp::{
        SessionState,
        rewards::blend::{LOG_TARGET, RewardsParameters, target_session::TargetSessionState},
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

    #[expect(
        clippy::unused_self,
        reason = "TODO: Create proof verifiers using `self.leader_inputs`"
    )]
    pub fn finalize(
        &self,
        last_active_session_state: &SessionState,
        next_session_first_epoch_state: &EpochState,
        settings: &RewardsParameters,
    ) -> CurrentSessionTrackerFinalization {
        if last_active_session_state.declarations.size()
            < settings.minimum_network_size.get() as usize
        {
            debug!(target: LOG_TARGET, "Declaration count({}) is below minimum network size({}). Switching to WithoutTargetSession mode",
                last_active_session_state.declarations.size(),
                settings.minimum_network_size.get()
            );
            return CurrentSessionTrackerFinalization::WithoutTargetSession(Self::new(
                next_session_first_epoch_state,
                settings,
            ));
        }

        let providers = Self::providers(last_active_session_state);

        // TODO: Create proof verifiers using `self.leader_inputs`.

        let token_evaluation = settings.token_evaluation(
            providers.size() as u64,
        ).expect("evaluation parameters shouldn't overflow. panicking since we can't process the new session");

        CurrentSessionTrackerFinalization::WithTargetSession {
            target_session_state: TargetSessionState::new(
                last_active_session_state.session_n,
                providers,
                token_evaluation,
            ),
            current_session_state: CurrentSessionState::new(SessionRandomness::new(
                last_active_session_state.session_n + 1,
                &next_session_first_epoch_state.nonce,
            )),
            current_session_tracker: Self::new(next_session_first_epoch_state, settings),
        }
    }

    fn providers(session_state: &SessionState) -> HashTrieMapSync<ProviderId, PublicKey> {
        session_state
            .declarations
            .values()
            .map(|declaration| (declaration.provider_id, declaration.zk_id))
            .collect()
    }

    #[cfg(test)]
    pub const fn num_leader_inputs(&self) -> usize {
        self.leader_inputs.len()
    }
}

pub enum CurrentSessionTrackerFinalization {
    WithTargetSession {
        target_session_state: TargetSessionState,
        current_session_state: CurrentSessionState,
        current_session_tracker: CurrentSessionTracker,
    },
    WithoutTargetSession(CurrentSessionTracker),
}
