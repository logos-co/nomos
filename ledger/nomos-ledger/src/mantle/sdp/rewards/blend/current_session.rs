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
    /// Collecting leader inputs derived from epoch states seen in the current
    /// session. These will be used to create proof verifiers after the next
    /// session update.
    leader_inputs: HashTrieMapSync<Epoch, LeaderInputs>,
}

impl CurrentSessionTracker {
    pub fn new(first_epoch_state: &EpochState, settings: &RewardsParameters) -> Self {
        Self {
            leader_inputs: std::iter::once((
                first_epoch_state.epoch,
                settings.leader_inputs(first_epoch_state),
            ))
            .collect(),
        }
    }

    pub fn collect_epoch(&self, epoch_state: &EpochState, settings: &RewardsParameters) -> Self {
        Self {
            leader_inputs: self
                .leader_inputs
                .insert(epoch_state.epoch, settings.leader_inputs(epoch_state)),
        }
    }

    /// Finalizes the current session tracker.
    ///
    /// It returns [`CurrentSessionTrackerOutput::WithTargetSession`] by
    /// creating a [`TargetSessionState`] using the collected information,
    /// if the network size of the new target session is not below the
    /// minimum required. Otherwise, it returns
    /// [`CurrentSessionTrackerOutput::WithoutTargetSession`].
    #[expect(
        clippy::unused_self,
        reason = "TODO: Create proof verifiers using `self.leader_inputs`"
    )]
    pub fn finalize(
        &self,
        last_active_session_state: &SessionState,
        next_session_first_epoch_state: &EpochState,
        settings: &RewardsParameters,
    ) -> CurrentSessionTrackerOutput {
        if last_active_session_state.declarations.size()
            < settings.minimum_network_size.get() as usize
        {
            debug!(target: LOG_TARGET, "Declaration count({}) is below minimum network size({}). Switching to WithoutTargetSession mode",
                last_active_session_state.declarations.size(),
                settings.minimum_network_size.get()
            );
            return CurrentSessionTrackerOutput::WithoutTargetSession(Self::new(
                next_session_first_epoch_state,
                settings,
            ));
        }

        let providers = Self::providers(last_active_session_state);

        // TODO: Create proof verifiers using `self.leader_inputs`.

        let token_evaluation = settings.token_evaluation(
            providers.size() as u64,
        ).expect("evaluation parameters shouldn't overflow. panicking since we can't process the new session");

        CurrentSessionTrackerOutput::WithTargetSession {
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
    pub fn epoch_count(&self) -> usize {
        self.leader_inputs.size()
    }
}

/// Result of finalizing the [`CurrentSessionTracker`].
pub enum CurrentSessionTrackerOutput {
    /// Target session has been built with the information collected by
    /// the current session tracker.
    /// Also, the new current session state and tracker have been initialized.
    WithTargetSession {
        target_session_state: TargetSessionState,
        current_session_state: CurrentSessionState,
        current_session_tracker: CurrentSessionTracker,
    },
    /// No target session has been built because the network size in the
    /// session is below the minimum required.
    WithoutTargetSession(CurrentSessionTracker),
}
