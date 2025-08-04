use std::hash::Hash;

use nomos_core::header::HeaderId;

use crate::BootstrapConfig;

pub fn choose_engine_state<NodeId>(
    lib_id: HeaderId,
    genesis_id: HeaderId,
    config: &BootstrapConfig<NodeId>,
) -> cryptarchia_engine::State
where
    NodeId: Clone + Eq + Hash,
{
    if lib_id == genesis_id || config.force_bootstrap {
        // TODO: Implement other criteria for bootstrapping
        //       - Offline grace period: https://github.com/logos-co/nomos/issues/1453
        //       - Checkpoint: https://github.com/logos-co/nomos/issues/1454
        cryptarchia_engine::State::Bootstrapping
    } else {
        cryptarchia_engine::State::Online
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, time::Duration};

    use super::*;
    use crate::IbdConfig;

    #[test]
    fn with_genesis_lib() {
        let state = choose_engine_state([0u8; 32].into(), [0u8; 32].into(), &config(false));
        assert_eq!(state, cryptarchia_engine::State::Bootstrapping);
    }

    #[test]
    fn with_non_genesis_lib() {
        let state = choose_engine_state([3u8; 32].into(), [0u8; 32].into(), &config(false));
        assert_eq!(state, cryptarchia_engine::State::Online);
    }

    #[test]
    fn with_force_bootstrap() {
        let state = choose_engine_state([3u8; 32].into(), [0u8; 32].into(), &config(true));
        assert_eq!(state, cryptarchia_engine::State::Bootstrapping);
    }

    fn config(force_bootstrap: bool) -> BootstrapConfig<usize> {
        BootstrapConfig {
            prolonged_bootstrap_period: Duration::ZERO,
            force_bootstrap,
            ibd: IbdConfig {
                peers: HashSet::new(),
                delay_before_new_download: Duration::from_millis(1),
            },
        }
    }
}
