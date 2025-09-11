use cryptarchia_engine::Slot;
use groth16::Fr;
use nomos_core::{
    mantle::{keys::SecretKey, Utxo},
    proofs::leader_proof::Risc0LeaderProof,
};
use nomos_ledger::{EpochState, UtxoTree};
use nomos_proof_statements::leadership::{LeaderPrivate, LeaderPublic};
use num_bigint::BigUint;
use serde::{Deserialize, Serialize};

/// TODO: this is a temporary solution until we have a proper wallet
/// implementation. Most notably, it can't track when initial notes are spent
/// and moved
#[derive(Clone)]
pub struct Leader {
    utxos: Vec<Utxo>,
    sk: SecretKey,
    config: nomos_ledger::Config,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LeaderConfig {
    pub utxos: Vec<Utxo>,
    // this is common to every note
    pub sk: SecretKey,
}

impl Leader {
    pub const fn new(utxos: Vec<Utxo>, sk: SecretKey, config: nomos_ledger::Config) -> Self {
        Self { utxos, sk, config }
    }

    #[expect(
        clippy::cognitive_complexity,
        reason = "TODO: Address this at some point"
    )]
    pub async fn build_proof_for(
        &self,
        aged_tree: &UtxoTree,
        latest_tree: &UtxoTree,
        epoch_state: &EpochState,
        slot: Slot,
    ) -> Option<Risc0LeaderProof> {
        for utxo in &self.utxos {
            let Some(_aged_witness) = aged_tree.witness(&utxo.id()) else {
                continue;
            };
            let Some(_latest_witness) = latest_tree.witness(&utxo.id()) else {
                continue;
            };

            let note_id: Fr = BigUint::from(1u8).into(); // placeholder for note ID, replace after mantle notes format update

            let public_inputs = LeaderPublic::new(
                aged_tree.root(),
                latest_tree.root(),
                [1; 32], // placeholder for entropy
                *epoch_state.nonce(),
                slot.into(),
                self.config.consensus_config.active_slot_coeff,
                epoch_state.total_stake(),
            );

            if public_inputs.check_winning(utxo.note.value, note_id, *self.sk.as_fr()) {
                tracing::debug!(
                    "leader for slot {:?}, {:?}/{:?}",
                    slot,
                    utxo.note.value,
                    epoch_state.total_stake()
                );

                let private_inputs = LeaderPrivate {
                    value: utxo.note.value,
                    note_id,
                    sk: *self.sk.as_fr(),
                };
                let res = tokio::task::spawn_blocking(move || {
                    Risc0LeaderProof::prove(
                        public_inputs,
                        &private_inputs,
                        risc0_zkvm::default_prover().as_ref(),
                    )
                })
                .await;
                match res {
                    Ok(Ok(proof)) => return Some(proof),
                    Ok(Err(e)) => {
                        tracing::error!("Failed to build proof: {:?}", e);
                    }
                    Err(e) => {
                        tracing::error!("Failed to wait thread to build proof: {:?}", e);
                    }
                }
            } else {
                tracing::trace!(
                    "Not a leader for slot {:?}, {:?}/{:?}",
                    slot,
                    utxo.note.value,
                    epoch_state.total_stake()
                );
            }
        }

        None
    }

    pub(crate) fn utxos(&self) -> &[Utxo] {
        &self.utxos
    }
}
