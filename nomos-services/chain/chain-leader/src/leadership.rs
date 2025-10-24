use cryptarchia_engine::{Epoch, Slot};
use nomos_core::{
    mantle::{
        Utxo,
        keys::{PublicKey, SecretKey},
        ops::leader_claim::VoucherCm,
    },
    proofs::leader_proof::{Groth16LeaderProof, LeaderPrivate, LeaderPublic},
};
use nomos_ledger::{EpochState, UtxoTree};
use serde::{Deserialize, Serialize};
use tokio::sync::watch::Sender;

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
    /// Return a leadership proof if the current slot is a winning one, and
    /// notifies consumers of winning slot info.
    ///
    /// If the slot is not a winning one, it returns `None` and no consumer is
    /// notified.
    pub async fn build_proof_for(
        &self,
        utxos: &[Utxo],
        latest_tree: &UtxoTree,
        epoch_state: &EpochState,
        slot: Slot,
        winning_pol_info_notifier: &WinningPoLSlotNotifier<'_>,
    ) -> Option<Groth16LeaderProof> {
        for utxo in utxos {
            let public_inputs = public_inputs_for_slot(epoch_state, slot, latest_tree);

            let note_id = utxo.id().0;
            let secret_key = self.slot_secret_key(slot);

            #[cfg(feature = "pol-dev-mode")]
            let winning = public_inputs.check_winning_dev(
                utxo.note.value,
                note_id,
                *secret_key.as_fr(),
                self.config.consensus_config.active_slot_coeff,
            );
            #[cfg(not(feature = "pol-dev-mode"))]
            let winning =
                public_inputs.check_winning(utxo.note.value, note_id, *secret_key.as_fr());

            if winning {
                tracing::debug!(
                    "leader for slot {:?}, {:?}/{:?}",
                    slot,
                    utxo.note.value,
                    epoch_state.total_stake()
                );

                let private_inputs = self.private_inputs_for_winning_utxo_and_slot(
                    utxo,
                    epoch_state,
                    public_inputs,
                    latest_tree,
                );

                winning_pol_info_notifier.notify_about_winning_slot(
                    private_inputs.clone(),
                    secret_key,
                    epoch_state.epoch,
                    slot,
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

    fn private_inputs_for_winning_utxo_and_slot(
        &self,
        utxo: &Utxo,
        // TODO: Use aged tree to compute `aged_path`
        epoch_state: &EpochState,
        public_inputs: LeaderPublic,
        // TODO: Use latest tree to compute `latest_path`
        _latest_tree: &UtxoTree,
    ) -> LeaderPrivate {
        // TODO: Get the actual witness paths and leader key
        let aged_path = Vec::new(); // Placeholder for aged path, aged UTXO tree is included in `EpochState`.
        let latest_path = Vec::new();
        let slot_secret = *self.sk.as_fr();
        let starting_slot = self
            .config
            .epoch_config
            .starting_slot(&epoch_state.epoch, self.config.base_period_length())
            .into();
        let leader_pk = ed25519_dalek::VerifyingKey::from_bytes(&[0; 32]).unwrap(); // TODO: get actual leader public key

        LeaderPrivate::new(
            public_inputs,
            *utxo,
            &aged_path,
            &latest_path,
            slot_secret,
            starting_slot,
            &leader_pk,
        )
    }

    fn slot_secret_key(&self, _slot: Slot) -> SecretKey {
        self.sk.clone()
    }
}

fn public_inputs_for_slot(
    epoch_state: &EpochState,
    slot: Slot,
    latest_tree: &UtxoTree,
) -> LeaderPublic {
    LeaderPublic::new(
        epoch_state.utxos.root(),
        latest_tree.root(),
        epoch_state.nonce,
        slot.into(),
        epoch_state.total_stake(),
    )
}

/// Process every tick and reacts to the very first one received and the first
/// one of every new epoch.
///
/// Reacting to a tick means pre-calculating the winning slots for the epoch and
/// notifying all consumers via the provided sender channel.
pub struct WinningPoLSlotNotifier<'service> {
    leader: &'service Leader,
    sender: &'service Sender<Option<(LeaderPrivate, SecretKey, Epoch)>>,
    /// Keeps track of the last processed epoch, if any, and for it the first
    /// winning slot that was pre-computed, if any.
    last_processed_epoch_and_found_first_winning_slot: Option<(Epoch, Option<Slot>)>,
}

impl<'service> WinningPoLSlotNotifier<'service> {
    pub(super) const fn new(
        leader: &'service Leader,
        sender: &'service Sender<Option<(LeaderPrivate, SecretKey, Epoch)>>,
    ) -> Self {
        Self {
            leader,
            sender,
            last_processed_epoch_and_found_first_winning_slot: None,
        }
    }

    /// It processes a new unprocessed epoch, and sends over the channel the
    /// first identified winning slot for this epoch, if any.
    pub(super) fn process_epoch(&mut self, utxos: &[Utxo], epoch_state: &EpochState) {
        if let Some((last_processed_epoch, _)) =
            self.last_processed_epoch_and_found_first_winning_slot
        {
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

        self.check_epoch_winning_utxos(utxos, epoch_state);
    }

    fn check_epoch_winning_utxos(&mut self, utxos: &[Utxo], epoch_state: &EpochState) {
        let slots_per_epoch = self.leader.config.epoch_length();
        let epoch_starting_slot: u64 = self
            .leader
            .config
            .epoch_config
            .starting_slot(&epoch_state.epoch, self.leader.config.base_period_length())
            .into();
        // Not used to check if a slot wins the lottery.
        let latest_tree = UtxoTree::new();

        let mut first_winning_slot: Option<Slot> = None;
        for utxo in utxos {
            let note_id = utxo.id().0;

            for offset in 0..slots_per_epoch {
                let slot = epoch_starting_slot
                    .checked_add(offset)
                    .expect("Slot calculation overflow.");
                let secret_key = self.leader.slot_secret_key(slot.into());

                let public_inputs = public_inputs_for_slot(epoch_state, slot.into(), &latest_tree);
                if !public_inputs.check_winning(utxo.note.value, note_id, *secret_key.as_fr()) {
                    continue;
                }
                tracing::debug!("Found winning utxo with ID {:?} for slot {slot}", utxo.id());

                let leader_private = self.leader.private_inputs_for_winning_utxo_and_slot(
                    utxo,
                    epoch_state,
                    public_inputs,
                    &latest_tree,
                );

                if let Err(err) = self.sender.send(Some((
                    leader_private,
                    secret_key.clone(),
                    epoch_state.epoch,
                ))) {
                    tracing::error!(
                        "Failed to send pre-calculated PoL winning slots to receivers. Error: {err:?}"
                    );
                } else {
                    // We stop the iteration as soon as the first winning slot for this epoch is
                    // found and was successfully communicated to consumers.
                    first_winning_slot = Some(slot.into());
                    break;
                }
            }
        }
        self.last_processed_epoch_and_found_first_winning_slot =
            Some((epoch_state.epoch, first_winning_slot));
    }

    /// Send the information about a winning slot to consumers.
    ///
    /// No check is performed on whether the slot is actually a winning one.
    pub(super) fn notify_about_winning_slot(
        &self,
        private_inputs: LeaderPrivate,
        secret_key: SecretKey,
        epoch: Epoch,
        slot: Slot,
    ) {
        // If we are trying to notify about the first winning slot that we already
        // pre-computed, ignore it.
        if let Some((_, Some(first_epoch_winning_slot))) =
            self.last_processed_epoch_and_found_first_winning_slot
            && first_epoch_winning_slot == slot
        {
            tracing::warn!(
                "Skipping notifying about winning slot {slot:?} because it was already processed"
            );
            return;
        }

        if let Err(err) = self.sender.send(Some((private_inputs, secret_key, epoch))) {
            tracing::error!(
                "Failed to send pre-calculated PoL winning slots to receivers. Error: {err:?}"
            );
        }
    }
}
