use cryptarchia_engine::Slot;
use groth16::Fr;
use nomos_core::{
    mantle::{Utxo, keys::SecretKey, ops::leader_claim::VoucherCm},
    proofs::leader_proof::{Groth16LeaderProof, LeaderPrivate, LeaderPublic},
};
use nomos_ledger::{EpochState, UtxoTree};
use num_bigint::BigUint;
use serde::{Deserialize, Serialize};

/// TODO: this is a temporary solution until we have a proper wallet
/// implementation. Most notably, it can't track when initial notes are spent
/// and moved
#[derive(Clone)]
pub struct Leader {
    utxos: Vec<Utxo>,
    sk: SecretKey,
    #[cfg_attr(
        not(feature = "pol-dev-mode"),
        expect(dead_code, reason = "Config is only used in pol-dev-mode")
    )]
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
    ) -> Option<Groth16LeaderProof> {
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
                epoch_state.nonce,
                slot.into(),
                epoch_state.total_stake(),
            );

            #[cfg(feature = "pol-dev-mode")]
            let winning = public_inputs.check_winning_dev(
                utxo.note.value,
                note_id,
                *self.sk.as_fr(),
                self.config.consensus_config.active_slot_coeff,
            );
            #[cfg(not(feature = "pol-dev-mode"))]
            let winning = public_inputs.check_winning(utxo.note.value, note_id, *self.sk.as_fr());

            if winning {
                tracing::debug!(
                    "leader for slot {:?}, {:?}/{:?}",
                    slot,
                    utxo.note.value,
                    epoch_state.total_stake()
                );

                // TODO: Get the actual witness paths and leader key
                let aged_path = Vec::new(); // Placeholder for aged path
                let latest_path = Vec::new();
                let slot_secret = *self.sk.as_fr();
                let starting_slot = 0u64; // TODO: get actual starting slot
                let leader_pk = ed25519_dalek::VerifyingKey::from_bytes(&[0; 32]).unwrap(); // TODO: get actual leader public key

                let private_inputs = LeaderPrivate::new(
                    public_inputs,
                    *utxo,
                    &aged_path,
                    &latest_path,
                    slot_secret,
                    starting_slot,
                    &leader_pk,
                );
                let res = tokio::task::spawn_blocking(move || {
                    Groth16LeaderProof::prove(
                        &private_inputs,
                        VoucherCm::default(), // TODO: use actual voucher commitment
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
}
