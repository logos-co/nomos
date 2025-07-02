use std::num::NonZero;

use chain_service::LeaderConfig;
use cryptarchia_engine::EpochConfig;
use nomos_core::mantle::{keys::SecretKey, Note, Utxo};
use nomos_ledger::LedgerState;

#[derive(Clone)]
pub struct ConsensusParams {
    pub n_participants: usize,
    pub security_param: NonZero<u32>,
    pub active_slot_coeff: f64,
}

impl ConsensusParams {
    #[must_use]
    pub const fn default_for_participants(n_participants: usize) -> Self {
        Self {
            n_participants,
            // by setting the slot coeff to 1, we also increase the probability of multiple blocks
            // (forks) being produced in the same slot (epoch). Setting the security
            // parameter to some value > 1 ensures nodes have some time to sync before
            // deciding on the longest chain.
            security_param: NonZero::new(10).unwrap(),
            // a block should be produced (on average) every slot
            active_slot_coeff: 0.9,
        }
    }
}

/// General consensus configuration for a chosen participant, that later could
/// be converted into a specific service or services configuration.
#[derive(Clone)]
pub struct GeneralConsensusConfig {
    pub leader_config: LeaderConfig,
    pub ledger_config: nomos_ledger::Config,
    pub genesis_state: LedgerState,
}

#[must_use]
pub fn create_consensus_configs(
    ids: &[[u8; 32]],
    consensus_params: &ConsensusParams,
) -> Vec<GeneralConsensusConfig> {
    let sks = ids
        .iter()
        .map(|&id| {
            let mut sk = [0; 16];
            sk.copy_from_slice(&id[0..16]);
            SecretKey::from(sk)
        })
        .collect::<Vec<_>>();

    let utxos = sks
        .iter()
        .map(|_| Utxo {
            note: Note::new(1, [0; 32].into()), // TODO: replace with proper public key
            tx_hash: [0; 32].into(),
            output_index: 0,
        })
        .collect::<Vec<_>>();
    let genesis_state = LedgerState::from_utxos(utxos.clone());
    let ledger_config = nomos_ledger::Config {
        epoch_config: EpochConfig {
            epoch_stake_distribution_stabilization: NonZero::new(3).unwrap(),
            epoch_period_nonce_buffer: NonZero::new(3).unwrap(),
            epoch_period_nonce_stabilization: NonZero::new(4).unwrap(),
        },
        consensus_config: cryptarchia_engine::Config {
            security_param: consensus_params.security_param,
            active_slot_coeff: consensus_params.active_slot_coeff,
        },
    };

    utxos
        .into_iter()
        .zip(sks)
        .map(|(utxo, sk)| GeneralConsensusConfig {
            leader_config: LeaderConfig {
                utxos: vec![utxo],
                sk,
            },
            ledger_config,
            genesis_state: genesis_state.clone(),
        })
        .collect()
}
