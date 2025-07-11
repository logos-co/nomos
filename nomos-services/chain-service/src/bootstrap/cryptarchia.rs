use cryptarchia_engine::{Boostrapping, Online};
use nomos_core::header::HeaderId;
use nomos_ledger::LedgerState;

use crate::{bootstrap::config::BootstrapConfig, Cryptarchia};

/// A wrapper that initializes [`Cryptarchia`] by selecting the appropriate
/// initial [`cryptarchia_engine::CryptarchiaState`].
pub enum InitialCryptarchia {
    Bootstrapping(Cryptarchia<Boostrapping>),
    Online(Cryptarchia<Online>),
}

impl InitialCryptarchia {
    /// Initialize [`Cryptarchia`] with the proper
    /// [`cryptarchia_engine::CryptarchiaState`].
    ///
    /// In the following cases, the [`cryptarchia_engine::Boostrapping`] is
    /// selected.
    /// - If the node is starting with LIB set to the genesis or a checkpoint.
    /// - If the node is restarting after being offline longer than the offline
    ///   grace period.
    /// - If the [`BootstrapConfig::force_bootstrap`] is true.
    ///
    /// Otherwise, the [`cryptarchia_engine::Online`] fork choice rule is
    /// selected.
    pub fn new(
        lib: HeaderId,
        lib_ledger_state: LedgerState,
        ledger_config: nomos_ledger::Config,
        genesis_id: HeaderId,
        config: &BootstrapConfig,
    ) -> Self {
        if lib == genesis_id || config.force_bootstrap {
            return Self::Bootstrapping(Cryptarchia::from_lib(
                lib,
                lib_ledger_state,
                ledger_config,
            ));
        }

        // TODO: Implement other criteria for bootstrapping.

        Self::Online(Cryptarchia::from_lib(lib, lib_ledger_state, ledger_config))
    }

    pub fn lib(&self) -> HeaderId {
        match self {
            Self::Bootstrapping(cryptarchia) => cryptarchia.consensus.lib_branch().id(),
            Self::Online(cryptarchia) => cryptarchia.consensus.lib_branch().id(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{iter::empty, num::NonZero, time::Duration};

    use super::*;

    #[test]
    fn initial_cryptarchia_with_genesis_lib() {
        let genesis_id = HeaderId::from([0u8; 32]);
        let cryptarchia = InitialCryptarchia::new(
            genesis_id, // Genesis LIB
            LedgerState::from_utxos(empty()),
            ledger_config(),
            genesis_id,
            &BootstrapConfig {
                prolonged_bootstrap_period: Duration::from_secs(1),
                force_bootstrap: false,
            },
        );
        assert!(matches!(cryptarchia, InitialCryptarchia::Bootstrapping(_)));
    }

    #[test]
    fn initial_cryptarchia_with_non_genesis_lib() {
        let genesis_id = HeaderId::from([0u8; 32]);
        let cryptarchia = InitialCryptarchia::new(
            HeaderId::from([1u8; 32]), // Non-genesis LIB
            LedgerState::from_utxos(empty()),
            ledger_config(),
            genesis_id,
            &BootstrapConfig {
                prolonged_bootstrap_period: Duration::from_secs(1),
                force_bootstrap: false,
            },
        );
        assert!(matches!(cryptarchia, InitialCryptarchia::Online(_)));
    }

    #[test]
    fn initial_cryptarchia_with_force_bootstrap() {
        let genesis_id = HeaderId::from([0u8; 32]);
        let cryptarchia = InitialCryptarchia::new(
            HeaderId::from([1u8; 32]), // Non-genesis LIB
            LedgerState::from_utxos(empty()),
            ledger_config(),
            genesis_id,
            &BootstrapConfig {
                prolonged_bootstrap_period: Duration::from_secs(1),
                force_bootstrap: true,
            },
        );
        assert!(matches!(cryptarchia, InitialCryptarchia::Bootstrapping(_)));
    }

    fn ledger_config() -> nomos_ledger::Config {
        nomos_ledger::Config {
            epoch_config: cryptarchia_engine::EpochConfig {
                epoch_stake_distribution_stabilization: NonZero::new(1).unwrap(),
                epoch_period_nonce_buffer: NonZero::new(1).unwrap(),
                epoch_period_nonce_stabilization: NonZero::new(1).unwrap(),
            },
            consensus_config: cryptarchia_engine::Config {
                security_param: NonZero::new(1).unwrap(),
                active_slot_coeff: 1.0,
            },
        }
    }
}
