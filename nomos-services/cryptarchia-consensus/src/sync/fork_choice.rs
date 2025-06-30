use cryptarchia_engine::Boostrapping;
use nomos_core::header::HeaderId;

use crate::{wrapper::CryptarchiaWrapper, Cryptarchia};

/// Selects the fork choice rule, as defined in the bootstrap spec.
/// If the following conditions are met, the Bootstrapping rule is selected:
/// - If the node is starting with LIB set to the genesis block or a checkpoint
///   block.
/// - If the node is restarting after being offline longer than the "Offline
///   Grace Period".
/// - If the `--bootstrap` flag is set.
///
/// Otherwise, the Online rule is selected.
///
/// Since the fork choice rule can be switched only from Bootstrapping to
/// Online, this function accepts only `Cryptarchia<Boostrapping>`.
pub fn select_fork_choice_rule(
    cryptarchia: Cryptarchia<Boostrapping>,
    genesis_id: HeaderId,
) -> CryptarchiaWrapper {
    // Choose Bootstrapping if the node is starting with LIB set to the genesis
    // block. TODO: Also choose Bootstrapping if the node is starting with LIB
    // set to a checkpoint. https://www.notion.so/Cryptarchia-v1-Bootstrapping-Synchronization-1fd261aa09df81ac94b5fb6a4eff32a6?source=copy_link#1fd261aa09df8136b6ffd9a910da0a6b
    if cryptarchia.lib() == genesis_id {
        return CryptarchiaWrapper::Bootstrapping(cryptarchia);
    }

    // TODO: Choose Bootstrapping if a node is restarting
    // after being offline longer than "Offline Grace Period".
    // https://www.notion.so/Cryptarchia-v1-Bootstrapping-Synchronization-1fd261aa09df81ac94b5fb6a4eff32a6?source=copy_link#1fd261aa09df81cfaffef835bc6f0e68

    // TODO: Choose Bootstrapping if `--bootstrap` flag is set.
    // https://www.notion.so/Cryptarchia-v1-Bootstrapping-Synchronization-1fd261aa09df81ac94b5fb6a4eff32a6?source=copy_link#1fd261aa09df81e38821d6911f78dba3

    CryptarchiaWrapper::Online(cryptarchia.online())
}

#[cfg(test)]
mod tests {
    use nomos_ledger::LedgerState;

    use super::*;

    const GENESIS_ID: [u8; 32] = [0u8; 32];
    const K: u8 = 3;

    #[test]
    fn lib_is_genesis() {
        let cryptarchia = build_cryptarchia(GENESIS_ID.into(), 0);
        let cryptarchia = select_fork_choice_rule(cryptarchia, GENESIS_ID.into());
        assert!(matches!(cryptarchia, CryptarchiaWrapper::Bootstrapping(_)));
    }

    #[test]
    fn more_blocks_but_lib_is_still_genesis() {
        let cryptarchia = build_cryptarchia(GENESIS_ID.into(), K);
        let cryptarchia = select_fork_choice_rule(cryptarchia, GENESIS_ID.into());
        assert!(matches!(cryptarchia, CryptarchiaWrapper::Bootstrapping(_)));
    }

    #[test]
    fn lib_is_not_genesis() {
        let cryptarchia = build_cryptarchia([100u8; 32].into(), 0);
        let cryptarchia = select_fork_choice_rule(cryptarchia, GENESIS_ID.into());
        assert!(matches!(cryptarchia, CryptarchiaWrapper::Online(_)));
    }

    fn build_cryptarchia(lib: HeaderId, blocks: u8) -> Cryptarchia<Boostrapping> {
        let consensus_config = cryptarchia_engine::Config {
            security_param: u32::from(K).try_into().unwrap(),
            active_slot_coeff: 1.0,
        };
        let mut consensus = <cryptarchia_engine::Cryptarchia<HeaderId, Boostrapping>>::from_lib(
            lib,
            consensus_config,
        );

        let mut parent = lib;
        for i in 1..=blocks {
            let id = [i; 32].into();
            consensus = consensus
                .receive_block(id, parent, u64::from(i).into())
                .expect("Block must be added successfully");
            parent = id;
        }

        Cryptarchia {
            consensus,
            ledger: <nomos_ledger::Ledger<HeaderId>>::new(
                lib,
                LedgerState::from_utxos([]),
                nomos_ledger::Config {
                    epoch_config: cryptarchia_engine::EpochConfig {
                        epoch_stake_distribution_stabilization: 1.try_into().unwrap(),
                        epoch_period_nonce_buffer: 1.try_into().unwrap(),
                        epoch_period_nonce_stabilization: 1.try_into().unwrap(),
                    },
                    consensus_config,
                },
            ),
        }
    }
}
