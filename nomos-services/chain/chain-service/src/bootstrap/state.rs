use std::{hash::Hash, time::SystemTime};

use nomos_core::header::HeaderId;
use tracing::warn;

use crate::{
    bootstrap::LOG_TARGET, states::LastEngineState, BootstrapConfig, OfflineGracePeriodConfig,
};

pub fn choose_engine_state<NodeId>(
    lib_id: HeaderId,
    genesis_id: HeaderId,
    config: &BootstrapConfig<NodeId>,
    last_engine_state: Option<&LastEngineState>,
) -> cryptarchia_engine::State
where
    NodeId: Clone + Eq + Hash,
{
    if lib_id == genesis_id || config.force_bootstrap {
        return cryptarchia_engine::State::Bootstrapping;
    }

    if let Some(last_state) = last_engine_state {
        return check_offline_grace_period(last_state, &config.offline_grace_period);
    }

    // TODO: Implement other criteria for bootstrapping
    //       - Checkpoint: https://github.com/logos-co/nomos/issues/1454
    cryptarchia_engine::State::Online
}

fn check_offline_grace_period(
    last_state: &LastEngineState,
    config: &OfflineGracePeriodConfig,
) -> cryptarchia_engine::State {
    let now = SystemTime::now();
    match now.duration_since(last_state.timestamp) {
        Ok(elapsed) => {
            if elapsed > config.grace_period {
                // Node has been offline longer than grace period, force bootstrap
                cryptarchia_engine::State::Bootstrapping
            } else {
                // Within grace period, use the last known state
                last_state.state
            }
        }
        Err(e) => {
            warn!(
                target: LOG_TARGET,
                "Offline duration measurement failed. Be conservative and bootstrap: now:{now:?}, last_state_timestamp:{:?}, error:{e:?}",
                last_state.timestamp,
            );
            cryptarchia_engine::State::Bootstrapping
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, time::Duration};

    use super::*;
    use crate::IbdConfig;

    #[test]
    fn with_genesis_lib() {
        let state = choose_engine_state([0u8; 32].into(), [0u8; 32].into(), &config(false), None);
        assert_eq!(state, cryptarchia_engine::State::Bootstrapping);
    }

    #[test]
    fn with_non_genesis_lib() {
        let state = choose_engine_state([3u8; 32].into(), [0u8; 32].into(), &config(false), None);
        assert_eq!(state, cryptarchia_engine::State::Online);
    }

    #[test]
    fn with_force_bootstrap() {
        let state = choose_engine_state([3u8; 32].into(), [0u8; 32].into(), &config(true), None);
        assert_eq!(state, cryptarchia_engine::State::Bootstrapping);
    }

    #[test]
    fn with_offline_grace_period_exceeded() {
        let last_state = LastEngineState {
            timestamp: SystemTime::now() - Duration::from_secs(30 * 60), // 30 minutes ago
            state: cryptarchia_engine::State::Online,
        };
        let state = choose_engine_state(
            [3u8; 32].into(),
            [0u8; 32].into(),
            &config(false),
            Some(&last_state),
        );
        assert_eq!(state, cryptarchia_engine::State::Bootstrapping);
    }

    #[test]
    fn with_offline_grace_period_not_exceeded() {
        let last_state = LastEngineState {
            timestamp: SystemTime::now() - Duration::from_secs(10 * 60), // 10 minutes ago
            state: cryptarchia_engine::State::Online,
        };
        let state = choose_engine_state(
            [3u8; 32].into(),
            [0u8; 32].into(),
            &config(false),
            Some(&last_state),
        );
        assert_eq!(state, cryptarchia_engine::State::Online);
    }

    #[test]
    fn with_last_state_bootstrapping() {
        let last_state = LastEngineState {
            timestamp: SystemTime::now() - Duration::from_secs(5 * 60), // 5 minutes ago
            state: cryptarchia_engine::State::Bootstrapping,
        };
        let state = choose_engine_state(
            [3u8; 32].into(),
            [0u8; 32].into(),
            &config(false),
            Some(&last_state),
        );
        assert_eq!(state, cryptarchia_engine::State::Bootstrapping);
    }

    fn config(force_bootstrap: bool) -> BootstrapConfig<usize> {
        BootstrapConfig {
            prolonged_bootstrap_period: Duration::ZERO,
            force_bootstrap,
            offline_grace_period: OfflineGracePeriodConfig {
                grace_period: Duration::from_secs(20 * 60),
                state_recording_interval: Duration::from_secs(60),
            },
            ibd: IbdConfig {
                peers: HashSet::new(),
                delay_before_new_download: Duration::from_millis(1),
                last_blocks_with_blob_validation: 2u64.into(),
            },
        }
    }
}
