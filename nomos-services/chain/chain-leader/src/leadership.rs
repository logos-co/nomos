use cryptarchia_engine::{Epoch, Slot};
use ed25519_dalek::VerifyingKey;
use groth16::Fr;
use nomos_core::{
    mantle::{
        Utxo,
        keys::{PublicKey, SecretKey},
        ops::leader_claim::VoucherCm,
    },
    proofs::leader_proof::{Groth16LeaderProof, LeaderPrivate, LeaderPublic},
    utils::merkle::MerklePath,
};
use nomos_ledger::{EpochState, UtxoTree};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::Sender;

#[derive(Clone)]
pub struct Leader {
    sk: SecretKey,
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
                let slot_secret = self.secret_for_slot(slot);
                let starting_slot: u64 = self
                    .config
                    .epoch_config
                    .starting_slot(&epoch_state.epoch)
                    .into();
                let leader_pk = leader_pk();

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

    const fn secret_for_slot(&self, _slot: Slot) -> Fr {
        *self.sk.as_fr()
    }

    fn slot_secret_key(&self, _slot: Slot) -> SecretKey {
        self.sk.clone()
    }
}

fn leader_pk() -> VerifyingKey {
    VerifyingKey::from_bytes(&[0; 32]).unwrap() // TODO: get actual leader public key
}

const fn path_for_aged_utxo(_utxo: &Utxo) -> MerklePath<Fr> {
    MerklePath::new() // Placeholder for aged path
}

/// Process every tick and reacts to the very first one received and the first
/// one of every new epoch.
///
/// Reacting to a tick means pre-calculating the winning slots for the epoch and
/// notifying all consumers via the provided sender channel.
pub struct WinningPoLSlotNotifier<'service> {
    leader: &'service Leader,
    sender: &'service Sender<(LeaderPrivate, SecretKey, Epoch)>,
    last_processed_epoch: Option<Epoch>,
}

impl<'service> WinningPoLSlotNotifier<'service> {
    pub(super) const fn new(
        leader: &'service Leader,
        sender: &'service Sender<(LeaderPrivate, SecretKey, Epoch)>,
    ) -> Self {
        Self {
            leader,
            sender,
            last_processed_epoch: None,
        }
    }

    pub(super) fn process_epoch(&mut self, epoch_state: &EpochState) {
        if let Some(last_processed_epoch) = self.last_processed_epoch {
            if last_processed_epoch == epoch_state.epoch {
                tracing::trace!("Skipping already processed epoch.");
                return;
            } else if last_processed_epoch > epoch_state.epoch {
                tracing::error!(
                    "Received an epoch smaller than the last process one. This is invalid."
                );
                return;
            }
        }
        tracing::debug!("Processing new epoch: {:?}", epoch_state.epoch);

        self.check_epoch_winning_utxos(epoch_state);
    }

    fn check_epoch_winning_utxos(&mut self, epoch_state: &EpochState) {
        use groth16::Field as _;

        let slots_per_epoch = self.leader.config.epoch_length();
        let epoch_starting_slot: u64 = self
            .leader
            .config
            .epoch_config
            .starting_slot(&epoch_state.epoch)
            .into();
        let aged_root = epoch_state.utxos.root();
        let epoch_nonce = epoch_state.nonce();
        let total_stake = epoch_state.total_stake();
        // Not used to check if a slot wins the lottery.
        let latest_root = Fr::ZERO;

        for utxo in &self.leader.utxos {
            let note_id = utxo.note.id();
            for offset in 0..slots_per_epoch {
                let slot = epoch_starting_slot
                    .checked_add(offset)
                    .expect("Slot calculation overflow.");
                let secret_key = self.leader.slot_secret_key(slot.into());
                let leader_public =
                    LeaderPublic::new(aged_root, latest_root, *epoch_nonce, slot, total_stake);
                if !leader_public.check_winning(utxo.note.value, note_id, *secret_key.as_fr()) {
                    continue;
                }
                tracing::debug!("Found winning utxo with ID {:?} for slot {slot}", utxo.id());

                let aged_path = path_for_aged_utxo(utxo);
                let slot_secret = self.leader.secret_for_slot(slot.into());

                let leader_private = LeaderPrivate::new(
                    leader_public,
                    *utxo,
                    &aged_path,
                    &vec![],
                    slot_secret,
                    epoch_starting_slot,
                    &leader_pk(),
                );
                if let Err(err) =
                    self.sender
                        .send((leader_private, secret_key.clone(), epoch_state.epoch))
                {
                    tracing::error!(
                        "Failed to send pre-calculated PoL winning slots to receivers. Error: {err:?}"
                    );
                }
            }
        }
        self.last_processed_epoch = Some(epoch_state.epoch);
    }
}
