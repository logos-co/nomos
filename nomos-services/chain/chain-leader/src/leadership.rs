use cryptarchia_engine::{Epoch, Slot};
use groth16::Fr;
use nomos_blend_service::ProofOfLeadershipQuotaInputs;
use nomos_core::{
    mantle::{
        Utxo,
        keys::{PublicKey, SecretKey},
        ops::leader_claim::VoucherCm,
    },
    proofs::leader_proof::{Groth16LeaderProof, LeaderPrivate, LeaderPublic},
    utils::merkle::MerkleNode,
};
use nomos_ledger::{EpochState, UtxoTree};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::Sender;

#[derive(Clone)]
pub struct Leader {
    sk: SecretKey,
    #[cfg_attr(
        not(feature = "pol-dev-mode"),
        expect(dead_code, reason = "Config is only used in pol-dev-mode")
    )]
    config: nomos_ledger::Config,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LeaderConfig {
    pub pk: PublicKey,
    pub sk: SecretKey,
}

impl Leader {
    pub const fn new(sk: SecretKey, config: nomos_ledger::Config) -> Self {
        Self { sk, config }
    }

    #[expect(
        clippy::cognitive_complexity,
        reason = "TODO: Address this at some point"
    )]
    pub async fn build_proof_for(
        &self,
        utxos: &[Utxo],
        aged_tree: &UtxoTree,
        latest_tree: &UtxoTree,
        epoch_state: &EpochState,
        slot: Slot,
    ) -> Option<Groth16LeaderProof> {
        for utxo in utxos {
            let Some(_aged_witness) = aged_tree.witness(&utxo.id()) else {
                continue;
            };
            let Some(_latest_witness) = latest_tree.witness(&utxo.id()) else {
                continue;
            };

            let note_id = utxo.note.id();
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
                let aged_path = path_for_aged_utxo(utxo);
                let latest_path = Vec::new();
                let slot_secret = *self.sk.as_fr();
                let starting_slot: u64 = self
                    .config
                    .epoch_config
                    .starting_slot(&epoch_state.epoch)
                    .into();
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
                        private_inputs,
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

    pub fn notify_winning_slots(
        &self,
        epoch_state: &EpochState,
        sender: &Sender<(ProofOfLeadershipQuotaInputs, Epoch)>,
    ) {
        use groth16::Field as _;

        let slots_per_epoch = self.config.epoch_length();
        for utxo in &self.utxos {
            let note_id = utxo.note.id();
            let epoch_starting_slot: u64 = self
                .config
                .epoch_config
                .starting_slot(&epoch_state.epoch)
                .into();
            for offset in 0..slots_per_epoch {
                let slot = epoch_starting_slot
                    .checked_add(offset)
                    .expect("Slot calculation overflow.");
                let aged_root = epoch_state.utxos.root();
                // Not used to check if a slot wins the lottery.
                let latest_root = Fr::ZERO;
                let epoch_nonce = epoch_state.nonce();
                let total_stake = epoch_state.total_stake();
                let secret_key = *self.sk.as_fr();
                let leader_public =
                    LeaderPublic::new(aged_root, latest_root, *epoch_nonce, slot, total_stake);
                if leader_public.check_winning(utxo.note.value, note_id, secret_key) {
                    let aged_path = path_for_aged_utxo(utxo);
                    let aged_selector = aged_path
                        .iter()
                        .map(|n| matches!(n, MerkleNode::Right(_)))
                        .collect();
                    let slot_secret = secret_key;
                    let slot_secret_path = vec![]; // TODO: implement same logic as in `LeaderPrivate`.

                    let poq_private_pol_inputs = ProofOfLeadershipQuotaInputs {
                        aged_path: aged_path.into_iter().map(|n| *n.item()).collect(),
                        aged_selector,
                        note_value: utxo.note.value,
                        output_number: utxo.output_index as u64,
                        pol_secret_key: secret_key,
                        slot,
                        slot_secret,
                        slot_secret_path,
                        starting_slot: epoch_starting_slot,
                        transaction_hash: utxo.tx_hash.0,
                    };
                    if let Err(err) = sender.send((poq_private_pol_inputs, epoch_state.epoch)) {
                        tracing::error!(
                            "Failed to send pre-calculated PoL winning slots to receivers. Error: {err:?}"
                        );
                    }
                }
            }
        }
    }
}

const fn path_for_aged_utxo(_utxo: &Utxo) -> Vec<MerkleNode<Fr>> {
    Vec::new() // Placeholder for aged path
}
