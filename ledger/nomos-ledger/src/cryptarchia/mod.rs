use cryptarchia_engine::{Epoch, Slot};
use nomos_core::{
    crypto::{Digest as _, Hasher},
    mantle::{gas::GasConstants, AuthenticatedMantleTx, Note, NoteId, Utxo, Value},
    proofs::{leader_proof, zksig::ZkSignatureProof as _},
};
use nomos_proof_statements::{leadership::LeaderPublic, zksig::ZkSignaturePublic};

pub type UtxoTree = utxotree::UtxoTree<NoteId, Note, Hasher>;
use super::{Config, LedgerError};

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EpochState {
    // The epoch this snapshot is for
    pub epoch: Epoch,
    // value of the ledger nonce after 'epoch_period_nonce_buffer' slots from the beginning of the
    // epoch
    pub nonce: [u8; 32],
    // stake distribution snapshot taken at the beginning of the epoch
    // (in practice, this is equivalent to the utxos the are spendable at the beginning of the
    // epoch)
    pub utxos: UtxoTree,
    pub total_stake: Value,
}

impl EpochState {
    fn update_from_ledger(self, ledger: &LedgerState, config: &Config) -> Self {
        let nonce_snapshot_slot = config.nonce_snapshot(self.epoch);
        let nonce = if ledger.slot < nonce_snapshot_slot {
            ledger.nonce
        } else {
            self.nonce
        };

        let stake_snapshot_slot = config.stake_distribution_snapshot(self.epoch);
        let utxos = if ledger.slot < stake_snapshot_slot {
            ledger.utxos.clone()
        } else {
            self.utxos
        };
        Self {
            epoch: self.epoch,
            nonce,
            utxos,
            total_stake: self.total_stake,
        }
    }

    #[must_use]
    pub const fn epoch(&self) -> Epoch {
        self.epoch
    }

    #[must_use]
    pub const fn nonce(&self) -> &[u8; 32] {
        &self.nonce
    }

    #[must_use]
    pub const fn total_stake(&self) -> Value {
        self.total_stake
    }
}

/// Tracks bedrock transactions and minimal the state needed for consensus to
/// work.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Eq, PartialEq)]
pub struct LedgerState {
    // All available Unspent Transtaction Outputs (UTXOs) at the current slot
    pub utxos: UtxoTree,
    // randomness contribution
    pub nonce: [u8; 32],
    pub slot: Slot,
    // rolling snapshot of the state for the next epoch, used for epoch transitions
    pub next_epoch_state: EpochState,
    pub epoch_state: EpochState,
}

impl LedgerState {
    fn update_epoch_state<Id>(self, slot: Slot, config: &Config) -> Result<Self, LedgerError<Id>> {
        if slot <= self.slot {
            return Err(LedgerError::InvalidSlot {
                parent: self.slot,
                block: slot,
            });
        }

        // TODO: update once supply can change
        let total_stake = self.epoch_state.total_stake;
        let current_epoch = config.epoch(self.slot);
        let new_epoch = config.epoch(slot);

        // there are 3 cases to consider:
        // 1. we are in the same epoch as the parent state update the next epoch state
        // 2. we are in the next epoch use the next epoch state as the current epoch
        //    state and reset next epoch state
        // 3. we are in the next-next or later epoch: use the parent state as the epoch
        //    state and reset next epoch state
        if current_epoch == new_epoch {
            // case 1)
            let next_epoch_state = self
                .next_epoch_state
                .clone()
                .update_from_ledger(&self, config);
            Ok(Self {
                slot,
                next_epoch_state,
                ..self
            })
        } else if new_epoch == current_epoch + 1 {
            // case 2)
            let epoch_state = self.next_epoch_state.clone();
            let next_epoch_state = EpochState {
                epoch: new_epoch + 1,
                nonce: self.nonce,
                utxos: self.utxos.clone(),
                total_stake,
            };
            Ok(Self {
                slot,
                next_epoch_state,
                epoch_state,
                ..self
            })
        } else {
            // case 3)
            let epoch_state = EpochState {
                epoch: new_epoch,
                nonce: self.nonce,
                utxos: self.utxos.clone(),
                total_stake,
            };
            let next_epoch_state = EpochState {
                epoch: new_epoch + 1,
                nonce: self.nonce,
                utxos: self.utxos.clone(),
                total_stake,
            };
            Ok(Self {
                slot,
                next_epoch_state,
                epoch_state,
                ..self
            })
        }
    }

    fn try_apply_proof<LeaderProof, Id>(
        self,
        slot: Slot,
        proof: &LeaderProof,
        config: &Config,
    ) -> Result<Self, LedgerError<Id>>
    where
        LeaderProof: leader_proof::LeaderProof,
    {
        assert_eq!(config.epoch(slot), self.epoch_state.epoch);
        let public_inputs = LeaderPublic::new(
            self.aged_commitments().root(),
            self.latest_commitments().root(),
            proof.entropy(),
            self.epoch_state.nonce,
            slot.into(),
            config.consensus_config.active_slot_coeff,
            self.epoch_state.total_stake,
        );
        if !proof.verify(&public_inputs) {
            return Err(LedgerError::InvalidProof);
        }

        Ok(self)
    }

    pub fn try_apply_header<LeaderProof, Id>(
        self,
        slot: Slot,
        proof: &LeaderProof,
        config: &Config,
    ) -> Result<Self, LedgerError<Id>>
    where
        LeaderProof: leader_proof::LeaderProof,
    {
        Ok(self
            .update_epoch_state(slot, config)?
            .try_apply_proof(slot, proof, config)?
            .update_nonce(proof.entropy(), slot))
    }

    pub fn try_apply_tx<Id, Constants: GasConstants>(
        mut self,
        tx: impl AuthenticatedMantleTx,
    ) -> Result<(Self, Value), LedgerError<Id>> {
        let mut balance: u64 = 0;
        let mut pks: Vec<[u8; 32]> = vec![];
        let ledger_tx = &tx.mantle_tx().ledger_tx;
        for input in &ledger_tx.inputs {
            let note;
            (self.utxos, note) = self
                .utxos
                .remove(input)
                .map_err(|_| LedgerError::InvalidNote(*input))?;
            balance = balance
                .checked_add(note.value)
                .ok_or(LedgerError::Overflow)?;
            pks.push(note.pk.into());
        }

        if !tx.ledger_tx_proof().verify(&ZkSignaturePublic {
            pks,
            msg_hash: tx.hash().into(),
        }) {
            return Err(LedgerError::InvalidProof);
        }

        for utxo in ledger_tx.utxos() {
            let note = utxo.note;
            if note.value == 0 {
                return Err(LedgerError::ZeroValueNote);
            }
            balance = balance
                .checked_sub(note.value)
                .ok_or(LedgerError::InsufficientBalance)?;
            self.utxos = self.utxos.insert(utxo.id(), note).0;
        }

        balance = balance
            .checked_sub(tx.gas_cost::<Constants>())
            .ok_or(LedgerError::InsufficientBalance)?;

        Ok((self, balance))
    }

    fn update_nonce(self, contrib: [u8; 32], slot: Slot) -> Self {
        // constants and structure as defined in the Mantle spec:
        // https://www.notion.so/Cryptarchia-v1-Protocol-Specification-21c261aa09df810cb85eff1c76e5798c
        const EPOCH_NONCE_V1: &[u8] = b"EPOCH_NONCE_V1";
        let mut hasher = Hasher::new();
        hasher.update(EPOCH_NONCE_V1);
        hasher.update(self.nonce);
        hasher.update(contrib);
        hasher.update(slot.to_be_bytes());

        let nonce: [u8; 32] = hasher.finalize().into();
        Self { nonce, ..self }
    }

    #[must_use]
    pub const fn slot(&self) -> Slot {
        self.slot
    }

    #[must_use]
    pub const fn epoch_state(&self) -> &EpochState {
        &self.epoch_state
    }

    #[must_use]
    pub const fn next_epoch_state(&self) -> &EpochState {
        &self.next_epoch_state
    }

    #[must_use]
    pub const fn latest_commitments(&self) -> &UtxoTree {
        &self.utxos
    }

    #[must_use]
    pub const fn aged_commitments(&self) -> &UtxoTree {
        &self.epoch_state.utxos
    }

    pub fn from_utxos(utxos: impl IntoIterator<Item = Utxo>) -> Self {
        let utxos = utxos
            .into_iter()
            .map(|utxo| (utxo.id(), utxo.note))
            .collect::<UtxoTree>();
        let total_stake = utxos
            .utxos()
            .iter()
            .map(|(_, (note, _))| note.value)
            .sum::<Value>();
        Self {
            utxos: utxos.clone(),
            nonce: [0; 32],
            slot: 0.into(),
            next_epoch_state: EpochState {
                epoch: 1.into(),
                nonce: [0; 32],
                utxos: utxos.clone(),
                total_stake,
            },
            epoch_state: EpochState {
                epoch: 0.into(),
                nonce: [0; 32],
                utxos,
                total_stake,
            },
        }
    }
}

#[expect(
    clippy::missing_fields_in_debug,
    reason = "No epoch info in debug output."
)]
impl core::fmt::Debug for LedgerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LedgerState")
            .field("utxos root", &self.utxos.root())
            .field("nonce", &self.nonce)
            .field("slot", &self.slot)
            .finish()
    }
}

#[cfg(test)]
pub mod tests {
    use std::num::NonZero;

    use cryptarchia_engine::EpochConfig;
    use crypto_bigint::U256;
    use nomos_core::{
        mantle::{
            gas::MainnetGasConstants, ledger::Tx as LedgerTx, GasCost as _, MantleTx,
            SignedMantleTx, Transaction as _,
        },
        proofs::zksig::DummyZkSignature,
    };
    use rand::{thread_rng, RngCore as _};

    use super::*;
    use crate::{leader_proof::LeaderProof, Ledger};

    type HeaderId = [u8; 32];

    #[must_use]
    pub fn utxo() -> Utxo {
        let mut tx_hash = [0; 32];
        thread_rng().fill_bytes(&mut tx_hash);
        Utxo {
            tx_hash: tx_hash.into(),
            output_index: 0,
            note: Note::new(10000, [0; 32].into()),
        }
    }

    pub struct DummyProof(pub LeaderPublic);

    impl LeaderProof for DummyProof {
        fn verify(&self, public_inputs: &LeaderPublic) -> bool {
            &self.0 == public_inputs
        }

        fn entropy(&self) -> [u8; 32] {
            self.0.entropy
        }
    }

    fn update_ledger(
        ledger: &mut Ledger<HeaderId>,
        parent: HeaderId,
        slot: impl Into<Slot>,
        utxo: Utxo,
    ) -> Result<HeaderId, LedgerError<HeaderId>> {
        let slot = slot.into();
        let ledger_state = ledger
            .state(&parent)
            .unwrap()
            .clone()
            .cryptarchia_ledger
            .update_epoch_state::<HeaderId>(slot, ledger.config())
            .unwrap();
        let config = ledger.config();
        let id = make_id(parent, slot, utxo);
        let proof = generate_proof(&ledger_state, &utxo, slot, config);
        *ledger = ledger.try_update::<_, MainnetGasConstants>(
            id,
            parent,
            slot,
            &proof,
            std::iter::empty::<&SignedMantleTx>(),
        )?;
        Ok(id)
    }

    fn make_id(parent: HeaderId, slot: impl Into<Slot>, utxo: Utxo) -> HeaderId {
        Hasher::new()
            .chain_update(parent)
            .chain_update(slot.into().to_be_bytes())
            .chain_update(utxo.id().0)
            .finalize()
            .into()
    }

    // produce a proof for a note
    #[must_use]
    pub fn generate_proof(
        ledger_state: &LedgerState,
        utxo: &Utxo,
        slot: Slot,
        config: &Config,
    ) -> DummyProof {
        let latest_tree = ledger_state.latest_commitments();
        let aged_tree = ledger_state.aged_commitments();

        DummyProof(LeaderPublic::new(
            if aged_tree.contains(&utxo.id()) {
                aged_tree.root()
            } else {
                println!("Note not found in latest commitments, using zero root");
                [0; 32]
            },
            if latest_tree.contains(&utxo.id()) {
                latest_tree.root()
            } else {
                println!("Note not found in latest commitments, using zero root");
                [0; 32]
            },
            [1; 32],
            ledger_state.epoch_state.nonce,
            slot.into(),
            config.consensus_config.active_slot_coeff,
            ledger_state.epoch_state.total_stake,
        ))
    }

    #[must_use]
    pub const fn config() -> Config {
        Config {
            epoch_config: EpochConfig {
                epoch_stake_distribution_stabilization: NonZero::new(4).unwrap(),
                epoch_period_nonce_buffer: NonZero::new(3).unwrap(),
                epoch_period_nonce_stabilization: NonZero::new(3).unwrap(),
            },
            consensus_config: cryptarchia_engine::Config {
                security_param: NonZero::new(1).unwrap(),
                active_slot_coeff: 1.0,
            },
        }
    }

    #[must_use]
    pub fn genesis_state(utxos: &[Utxo]) -> LedgerState {
        let total_stake = utxos.iter().map(|u| u.note.value).sum();
        let utxos = utxos
            .iter()
            .map(|utxo| (utxo.id(), utxo.note))
            .collect::<UtxoTree>();
        LedgerState {
            utxos: utxos.clone(),
            nonce: [0; 32],
            slot: 0.into(),
            next_epoch_state: EpochState {
                epoch: 1.into(),
                nonce: [0; 32],
                utxos: utxos.clone(),
                total_stake,
            },
            epoch_state: EpochState {
                epoch: 0.into(),
                nonce: [0; 32],
                utxos,
                total_stake,
            },
        }
    }

    fn full_ledger_state(cryptarchia_ledger: LedgerState) -> crate::LedgerState {
        crate::LedgerState {
            cryptarchia_ledger,
            mantle_ledger: (),
        }
    }

    fn ledger(utxos: &[Utxo]) -> (Ledger<HeaderId>, HeaderId) {
        let genesis_state = genesis_state(utxos);
        (
            Ledger::new([0; 32], full_ledger_state(genesis_state), config()),
            [0; 32],
        )
    }

    fn apply_and_add_utxo(
        ledger: &mut Ledger<HeaderId>,
        parent: HeaderId,
        slot: impl Into<Slot>,
        utxo_proof: Utxo,
        utxo_add: Utxo,
    ) -> HeaderId {
        let id = update_ledger(ledger, parent, slot, utxo_proof).unwrap();
        // we still don't have transactions, so the only way to add a commitment to
        // spendable commitments and test epoch snapshotting is by doing this
        // manually
        let mut block_state = ledger.states[&id].clone().cryptarchia_ledger;
        block_state.utxos = block_state.utxos.insert(utxo_add.id(), utxo_add.note).0;
        ledger.states.insert(id, full_ledger_state(block_state));
        id
    }

    #[test]
    fn test_ledger_state_allow_leadership_utxo_reuse() {
        let utxo = utxo();
        let (mut ledger, genesis) = ledger(&[utxo]);

        let h = update_ledger(&mut ledger, genesis, 1, utxo).unwrap();

        // reusing the same utxo for leadersip should be allowed
        update_ledger(&mut ledger, h, 2, utxo).unwrap();
    }

    #[test]
    fn test_ledger_state_uncommited_utxo() {
        let utxo_1 = utxo();
        let (mut ledger, genesis) = ledger(&[utxo()]);
        assert!(matches!(
            update_ledger(&mut ledger, genesis, 1, utxo_1),
            Err(LedgerError::InvalidProof),
        ));
    }

    #[test]
    fn test_epoch_transition() {
        let utxos = std::iter::repeat_with(utxo).take(4).collect::<Vec<_>>();
        let utxo_4 = utxo();
        let utxo_5 = utxo();
        let (mut ledger, genesis) = ledger(&utxos);

        // An epoch will be 10 slots long, with stake distribution snapshot taken at the
        // start of the epoch and nonce snapshot before slot 7

        let h_1 = update_ledger(&mut ledger, genesis, 1, utxos[0]).unwrap();
        assert_eq!(
            ledger.states[&h_1].cryptarchia_ledger.epoch_state.epoch,
            0.into()
        );

        let h_2 = update_ledger(&mut ledger, h_1, 6, utxos[1]).unwrap();

        let h_3 = apply_and_add_utxo(&mut ledger, h_2, 9, utxos[2], utxo_4);

        // test epoch jump
        let h_4 = update_ledger(&mut ledger, h_3, 20, utxos[3]).unwrap();
        // nonce for epoch 2 should be taken at the end of slot 16, but in our case the
        // last block is at slot 9
        assert_eq!(
            ledger.states[&h_4].cryptarchia_ledger.epoch_state.nonce,
            ledger.states[&h_3].cryptarchia_ledger.nonce,
        );
        // stake distribution snapshot should be taken at the end of slot 9
        assert_eq!(
            ledger.states[&h_4].cryptarchia_ledger.epoch_state.utxos,
            ledger.states[&h_3].cryptarchia_ledger.utxos,
        );

        // nonce for epoch 1 should be taken at the end of slot 6
        update_ledger(&mut ledger, h_3, 10, utxos[3]).unwrap();
        let h_5 = apply_and_add_utxo(&mut ledger, h_3, 10, utxos[3], utxo_5);
        assert_eq!(
            ledger.states[&h_5].cryptarchia_ledger.epoch_state.nonce,
            ledger.states[&h_2].cryptarchia_ledger.nonce,
        );

        let h_6 = update_ledger(&mut ledger, h_5, 20, utxos[3]).unwrap();
        // stake distribution snapshot should be taken at the end of slot 9, check that
        // changes in slot 10 are ignored
        assert_eq!(
            ledger.states[&h_6].cryptarchia_ledger.epoch_state.utxos,
            ledger.states[&h_3].cryptarchia_ledger.utxos,
        );
    }

    #[test]
    fn test_new_utxos_becoming_eligible_after_stake_distribution_stabilizes() {
        let utxo_1 = utxo();
        let utxo = utxo();

        let (mut ledger, genesis) = ledger(&[utxo]);

        // EPOCH 0
        // mint a new utxo to be used for leader elections in upcoming epochs
        let h_0_1 = apply_and_add_utxo(&mut ledger, genesis, 1, utxo, utxo_1);

        // the new utxo is not yet eligible for leader elections
        assert!(matches!(
            update_ledger(&mut ledger, h_0_1, 2, utxo_1),
            Err(LedgerError::InvalidProof),
        ));

        // EPOCH 1
        for i in 10..20 {
            // the newly minted utxo is still not eligible in the following epoch since the
            // stake distribution snapshot is taken at the beginning of the previous epoch
            assert!(matches!(
                update_ledger(&mut ledger, h_0_1, i, utxo_1),
                Err(LedgerError::InvalidProof),
            ));
        }

        // EPOCH 2
        // the utxo is finally eligible 2 epochs after it was first minted
        update_ledger(&mut ledger, h_0_1, 20, utxo_1).unwrap();
    }

    #[test]
    fn test_update_epoch_state_with_outdated_slot_error() {
        let utxo = utxo();
        let (ledger, genesis) = ledger(&[utxo]);

        let ledger_state = ledger.state(&genesis).unwrap().clone();
        let ledger_config = ledger.config();

        let slot = Slot::genesis() + 10;
        let ledger_state2 = ledger_state
            .cryptarchia_ledger
            .update_epoch_state::<HeaderId>(slot, ledger_config)
            .expect("Ledger needs to move forward");

        let slot2 = Slot::genesis() + 1;
        let update_epoch_err = ledger_state2
            .update_epoch_state::<HeaderId>(slot2, ledger_config)
            .err();

        // Time cannot flow backwards
        match update_epoch_err {
            Some(LedgerError::InvalidSlot { parent, block })
                if parent == slot && block == slot2 => {}
            _ => panic!("error does not match the LedgerError::InvalidSlot pattern"),
        }
    }

    #[test]
    fn test_invalid_aged_root_rejected() {
        let utxo = utxo();
        let (ledger, genesis) = ledger(&[utxo]);
        let ledger_state = ledger.state(&genesis).unwrap().clone().cryptarchia_ledger;
        let slot = Slot::genesis() + 1;
        let proof = DummyProof(LeaderPublic {
            aged_root: [1; 32], // Invalid aged root
            latest_root: ledger_state.latest_commitments().root(),
            epoch_nonce: ledger_state.epoch_state.nonce,
            slot: slot.into(),
            entropy: [1; 32],
            scaled_phi_approx: (U256::from(1u32), U256::from(1u32)),
        });
        let update_err = ledger_state
            .try_apply_proof::<_, ()>(slot, &proof, ledger.config())
            .err();

        assert_eq!(Some(LedgerError::InvalidProof), update_err);
    }

    #[test]
    fn test_invalid_latest_root_rejected() {
        let utxo = utxo();
        let (ledger, genesis) = ledger(&[utxo]);
        let ledger_state = ledger.state(&genesis).unwrap().clone().cryptarchia_ledger;
        let slot = Slot::genesis() + 1;
        let proof = DummyProof(LeaderPublic {
            aged_root: ledger_state.aged_commitments().root(),
            latest_root: [1; 32], // Invalid latest root
            epoch_nonce: ledger_state.epoch_state.nonce,
            slot: slot.into(),
            entropy: [1; 32],
            scaled_phi_approx: (U256::from(1u32), U256::from(1u32)),
        });
        let update_err = ledger_state
            .try_apply_proof::<_, ()>(slot, &proof, ledger.config())
            .err();

        assert_eq!(Some(LedgerError::InvalidProof), update_err);
    }

    fn create_tx(inputs: &[&Utxo], outputs: Vec<Note>) -> SignedMantleTx {
        let pks = inputs
            .iter()
            .map(|utxo| utxo.note.pk.into())
            .collect::<Vec<_>>();
        let inputs = inputs.iter().map(|utxo| utxo.id()).collect::<Vec<_>>();
        let ledger_tx = LedgerTx::new(inputs, outputs);
        let mantle_tx = MantleTx {
            ops: vec![],
            ledger_tx,
            execution_gas_price: 1,
            storage_gas_price: 1,
        };
        SignedMantleTx {
            ops_profs: vec![],
            ledger_tx_proof: DummyZkSignature::prove(ZkSignaturePublic {
                pks,
                msg_hash: mantle_tx.hash().into(),
            }),
            mantle_tx,
        }
    }

    #[test]
    fn test_tx_processing_valid_transaction() {
        let input_note = Note::new(10000, [1; 32].into());
        let input_utxo = Utxo {
            tx_hash: [1; 32].into(),
            output_index: 0,
            note: input_note,
        };

        let output_note1 = Note::new(4000, [2; 32].into());
        let output_note2 = Note::new(3000, [3; 32].into());

        let ledger_state = LedgerState::from_utxos([input_utxo]);
        let tx = create_tx(&[&input_utxo], vec![output_note1, output_note2]);

        let fees = tx.gas_cost::<MainnetGasConstants>();
        let (new_state, balance) = ledger_state
            .try_apply_tx::<(), MainnetGasConstants>(tx)
            .unwrap();

        assert_eq!(
            balance,
            input_note.value - output_note1.value - output_note2.value - fees
        );

        // Verify input was consumed
        assert!(!new_state.utxos.contains(&input_utxo.id()));

        // Verify outputs were created
        let mantle_tx = create_tx(&[&input_utxo], vec![output_note1, output_note2]);
        let output_utxo1 = mantle_tx.mantle_tx.ledger_tx.utxo_by_index(0).unwrap();
        let output_utxo2 = mantle_tx.mantle_tx.ledger_tx.utxo_by_index(1).unwrap();
        assert!(new_state.utxos.contains(&output_utxo1.id()));
        assert!(new_state.utxos.contains(&output_utxo2.id()));

        // The new outputs can be spent in future transactions
        let tx = create_tx(&[&output_utxo1, &output_utxo2], vec![]);
        let fees = tx.gas_cost::<MainnetGasConstants>();
        let (final_state, final_balance) = new_state
            .try_apply_tx::<(), MainnetGasConstants>(tx)
            .unwrap();
        assert_eq!(
            final_balance,
            output_note1.value + output_note2.value - fees
        );
        assert!(!final_state.utxos.contains(&output_utxo1.id()));
        assert!(!final_state.utxos.contains(&output_utxo2.id()));
    }

    #[test]
    fn test_tx_processing_invalid_input() {
        let input_note = Note::new(1000, [1; 32].into());
        let input_utxo = Utxo {
            tx_hash: [1; 32].into(),
            output_index: 0,
            note: input_note,
        };

        let non_existent_utxo_1 = Utxo {
            tx_hash: [1; 32].into(),
            output_index: 1,
            note: input_note,
        };

        let non_existent_utxo_2 = Utxo {
            tx_hash: [2; 32].into(),
            output_index: 0,
            note: input_note,
        };

        let non_existent_utxo_3 = Utxo {
            tx_hash: [1; 32].into(),
            output_index: 0,
            note: Note::new(999, [1; 32].into()),
        };

        let ledger_state = LedgerState::from_utxos([input_utxo]);

        let invalid_utxos = [
            non_existent_utxo_1,
            non_existent_utxo_2,
            non_existent_utxo_3,
        ];

        for non_existent_utxo in invalid_utxos {
            let tx = create_tx(&[&non_existent_utxo], vec![]);
            let result = ledger_state
                .clone()
                .try_apply_tx::<(), MainnetGasConstants>(tx);
            assert!(matches!(result, Err(LedgerError::InvalidNote(_))));
        }
    }

    #[test]
    fn test_tx_processing_insufficient_balance() {
        let input_note = Note::new(MainnetGasConstants::LEDGER_TX + 1, [1; 32].into());
        let input_utxo = Utxo {
            tx_hash: [1; 32].into(),
            output_index: 0,
            note: input_note,
        };

        let output_note = Note::new(1, [2; 32].into());

        let ledger_state = LedgerState::from_utxos([input_utxo]);
        let tx = create_tx(&[&input_utxo], vec![output_note, output_note]);

        let result = ledger_state
            .clone()
            .try_apply_tx::<(), MainnetGasConstants>(tx);
        assert!(matches!(result, Err(LedgerError::InsufficientBalance)));

        let tx = create_tx(&[&input_utxo], vec![output_note]);
        assert!(ledger_state
            .try_apply_tx::<(), MainnetGasConstants>(tx)
            .is_err());
    }

    #[test]
    fn test_tx_processing_insufficient_balance_with_gas() {
        let input_note = Note::new(1, [1; 32].into());
        let input_utxo = Utxo {
            tx_hash: [1; 32].into(),
            output_index: 0,
            note: input_note,
        };

        let output_note = Note::new(1, [2; 32].into());

        let ledger_state = LedgerState::from_utxos([input_utxo]);
        let tx = create_tx(&[&input_utxo], vec![output_note]);

        // input / output are balanced, but gas cost is not covered
        let result = ledger_state.try_apply_tx::<(), MainnetGasConstants>(tx);
        assert!(matches!(result, Err(LedgerError::InsufficientBalance)));
    }

    #[test]
    fn test_tx_processing_no_outputs() {
        let input_note = Note::new(10000, [1; 32].into());
        let input_utxo = Utxo {
            tx_hash: [1; 32].into(),
            output_index: 0,
            note: input_note,
        };

        let ledger_state = LedgerState::from_utxos([input_utxo]);
        let tx = create_tx(&[&input_utxo], vec![]);

        let fees = tx.gas_cost::<MainnetGasConstants>();
        let result = ledger_state.try_apply_tx::<(), MainnetGasConstants>(tx);
        assert!(result.is_ok());

        let (new_state, balance) = result.unwrap();
        assert_eq!(balance, 10000 - fees);

        // Verify input was consumed
        assert!(!new_state.utxos.contains(&input_utxo.id()));
    }

    #[test]
    fn test_tx_processing_overflow_protection() {
        let input_note1 = Note::new(u64::MAX, [1; 32].into());
        let input_note2 = Note::new(1, [2; 32].into());
        let input_utxo1 = Utxo {
            tx_hash: [1; 32].into(),
            output_index: 0,
            note: input_note1,
        };
        let input_utxo2 = Utxo {
            tx_hash: [2; 32].into(),
            output_index: 0,
            note: input_note2,
        };

        let output_note = Note::new(100, [3; 32].into());

        // adding both utxos together would overflow the total stake calculation
        let mut ledger_state = LedgerState::from_utxos([input_utxo1]);
        ledger_state.utxos = ledger_state
            .utxos
            .insert(input_utxo2.id(), input_utxo2.note)
            .0;
        let tx = create_tx(&[&input_utxo1, &input_utxo2], vec![output_note]);

        let result = ledger_state.try_apply_tx::<(), MainnetGasConstants>(tx);
        assert!(matches!(result, Err(LedgerError::Overflow)));
    }

    #[test]
    fn test_output_not_zero() {
        let input_utxo = Utxo {
            tx_hash: [1; 32].into(),
            output_index: 0,
            note: Note::new(10000, [1; 32].into()),
        };

        let ledger_state = LedgerState::from_utxos([input_utxo]);
        let tx = create_tx(&[&input_utxo], vec![Note::new(0, [2; 32].into())]);

        let result = ledger_state.try_apply_tx::<(), MainnetGasConstants>(tx);
        assert!(matches!(result, Err(LedgerError::ZeroValueNote)));
    }
}
