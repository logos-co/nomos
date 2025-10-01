use cryptarchia_engine::Slot;
use groth16::Fr;
use nomos_blend_service::{
    ProofOfLeadershipQuotaInputs, ProofOfQuota, ProofOfQuotaPrivateInputs, ProofOfQuotaPublicInputs,
};
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
use num_bigint::BigUint;
use serde::{Deserialize, Serialize};

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

    pub async fn is_slot_winning(&self, epoch_state: &EpochState, slot: Slot) -> bool {
        use groth16::Field as _;

        let public_inputs = ProofOfQuotaPublicInputs {
            core_quota: 0,
            core_root: Fr::ZERO,
            leader_quota: 1,
            pol_epoch_nonce: epoch_state.nonce,
            pol_ledger_aged: epoch_state.utxos.root(),
            session: 1,
            signing_key: [0; _].try_into().unwrap(),
            total_stake: epoch_state.total_stake,
        };

        // TODO: Get the actual witness paths and leader key
        let aged_path = Vec::new(); // Placeholder for aged path
        let aged_selector: Vec<bool> = aged_path
            .iter()
            .map(|n| matches!(n, MerkleNode::Right(_)))
            .collect();
        let aged_path: Vec<Fr> = aged_path.into_iter().map(|p| *p.item()).collect();
        let slot_secret = *self.sk.as_fr();
        let slot_secret_path = vec![]; // TODO: implement
        let starting_slot = 0u64; // TODO: get actual starting slot

        for utxo in &self.utxos {
            let private_inputs = ProofOfQuotaPrivateInputs::new_proof_of_leadership_quota_inputs(
                0,
                ProofOfLeadershipQuotaInputs {
                    aged_path: aged_path.clone(),
                    aged_selector: aged_selector.clone(),
                    note_value: utxo.note.value,
                    output_number: utxo.output_index as u64,
                    pol_secret_key: *self.sk.as_fr(),
                    slot: slot.into(),
                    slot_secret,
                    slot_secret_path: slot_secret_path.clone(),
                    starting_slot,
                    transaction_hash: utxo.tx_hash.0,
                },
            );
            if ProofOfQuota::new(&public_inputs, private_inputs).is_ok() {
                return true;
            }
        }
        false
    }
}
