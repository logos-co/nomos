use std::sync::LazyLock;

use cryptarchia_engine::{Epoch, Slot};
use groth16::{Field as _, Fr, fr_from_bytes};
use nomos_core::{
    crypto::{ZkDigest, ZkHasher},
    mantle::{AuthenticatedMantleTx, NoteId, Utxo, Value, gas::GasConstants},
    proofs::{
        leader_proof::{self, LeaderPublic},
        zksig::{ZkSignatureProof as _, ZkSignaturePublic},
    },
};

pub type UtxoTree = utxotree::UtxoTree<NoteId, Utxo, ZkHasher>;
use super::{Balance, Config, LedgerError};
use crate::mantle::locked_notes::LockedNotes;

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EpochState {
    // The epoch this snapshot is for
    pub epoch: Epoch,
    // value of the ledger nonce after 'epoch_period_nonce_buffer' slots from the beginning of the
    // epoch
    #[cfg_attr(feature = "serde", serde(with = "groth16::serde::serde_fr"))]
    pub nonce: Fr,
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
    pub const fn nonce(&self) -> &Fr {
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
    #[cfg_attr(feature = "serde", serde(with = "groth16::serde::serde_fr"))]
    pub nonce: Fr,
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
            self.epoch_state.nonce,
            slot.into(),
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
            .update_nonce(&proof.entropy(), slot))
    }

    pub fn try_apply_tx<Id, Constants: GasConstants>(
        mut self,
        locked_notes: &LockedNotes,
        tx: impl AuthenticatedMantleTx,
    ) -> Result<(Self, Balance), LedgerError<Id>> {
        let mut balance: i128 = 0;
        let mut pks: Vec<Fr> = vec![];
        let ledger_tx = &tx.mantle_tx().ledger_tx;
        for input in &ledger_tx.inputs {
            if locked_notes.contains(input) {
                return Err(LedgerError::LockedNote(*input));
            }
            let utxo;
            (self.utxos, utxo) = self
                .utxos
                .remove(input)
                .map_err(|_| LedgerError::InvalidNote(*input))?;
            balance = balance
                .checked_add(utxo.note.value.into())
                .ok_or(LedgerError::Overflow)?;
            pks.push(utxo.note.pk.into());
        }

        if !tx.ledger_tx_proof().verify(&ZkSignaturePublic {
            pks,
            msg_hash: tx.hash().into(),
        }) {
            return Err(LedgerError::InvalidProof);
        }

        for utxo in ledger_tx.utxos() {
            if utxo.note.value == 0 {
                return Err(LedgerError::ZeroValueNote);
            }
            balance = balance
                .checked_sub(utxo.note.value.into())
                .ok_or(LedgerError::Overflow)?;
            self.utxos = self.utxos.insert(utxo.id(), utxo).0;
        }

        Ok((self, balance))
    }

    fn update_nonce(self, contrib: &Fr, slot: Slot) -> Self {
        // constants and structure as defined in the Mantle spec:
        // https://www.notion.so/Cryptarchia-v1-Protocol-Specification-21c261aa09df810cb85eff1c76e5798c
        static EPOCH_NONCE_V1: LazyLock<Fr> =
            LazyLock::new(|| fr_from_bytes(b"EPOCH_NONCE_V1").unwrap());
        let mut hasher = ZkHasher::new();
        <ZkHasher as ZkDigest>::update(&mut hasher, &EPOCH_NONCE_V1);
        <ZkHasher as ZkDigest>::update(&mut hasher, &self.nonce);
        <ZkHasher as ZkDigest>::update(&mut hasher, contrib);
        <ZkHasher as ZkDigest>::update(&mut hasher, &Fr::from(u64::from(slot)));

        let nonce: Fr = hasher.finalize();
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
            .map(|utxo| (utxo.id(), utxo))
            .collect::<UtxoTree>();
        let total_stake = utxos
            .utxos()
            .iter()
            .map(|(_, (utxo, _))| utxo.note.value)
            .sum::<Value>();
        Self {
            utxos: utxos.clone(),
            nonce: Fr::ZERO,
            slot: 0.into(),
            next_epoch_state: EpochState {
                epoch: 1.into(),
                nonce: Fr::ZERO,
                utxos: utxos.clone(),
                total_stake,
            },
            epoch_state: EpochState {
                epoch: 0.into(),
                nonce: Fr::ZERO,
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
    use nomos_core::{
        crypto::{Digest as _, Hasher},
        mantle::{
            GasCost as _, MantleTx, Note, SignedMantleTx, Transaction as _,
            gas::MainnetGasConstants, ledger::Tx as LedgerTx, ops::leader_claim::VoucherCm,
        },
        proofs::zksig::DummyZkSignature,
    };
    use num_bigint::BigUint;
    use rand::{RngCore as _, thread_rng};

    use super::*;
    use crate::{Ledger, leader_proof::LeaderProof};

    type HeaderId = [u8; 32];

    #[must_use]
    pub fn utxo() -> Utxo {
        let tx_hash: Fr = BigUint::from(thread_rng().next_u64()).into();
        Utxo {
            tx_hash: tx_hash.into(),
            output_index: 0,
            note: Note::new(10000, Fr::from(BigUint::from(0u8)).into()),
        }
    }

    pub struct DummyProof {
        pub public: LeaderPublic,
        pub leader_key: ed25519_dalek::VerifyingKey,
        pub voucher_cm: VoucherCm,
    }

    impl LeaderProof for DummyProof {
        fn verify(&self, public_inputs: &LeaderPublic) -> bool {
            &self.public == public_inputs
        }

        fn entropy(&self) -> Fr {
            // For dummy proof, return zero entropy
            Fr::from(0u8)
        }

        fn leader_key(&self) -> &ed25519_dalek::VerifyingKey {
            &self.leader_key
        }

        fn voucher_cm(&self) -> &VoucherCm {
            &self.voucher_cm
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
        let id = make_id(parent, slot, utxo);
        let proof = generate_proof(&ledger_state, &utxo, slot);
        *ledger = ledger.try_update::<_, MainnetGasConstants>(
            id,
            parent,
            slot,
            &proof,
            VoucherCm::default(),
            std::iter::empty::<&SignedMantleTx>(),
        )?;
        Ok(id)
    }

    fn make_id(parent: HeaderId, slot: impl Into<Slot>, utxo: Utxo) -> HeaderId {
        Hasher::new()
            .chain_update(parent)
            .chain_update(slot.into().to_be_bytes())
            .chain_update(utxo.id().as_bytes())
            .finalize()
            .into()
    }

    // produce a proof for a note
    #[must_use]
    pub fn generate_proof(ledger_state: &LedgerState, utxo: &Utxo, slot: Slot) -> DummyProof {
        let latest_tree = ledger_state.latest_commitments();
        let aged_tree = ledger_state.aged_commitments();
        DummyProof {
            public: LeaderPublic::new(
                if aged_tree.contains(&utxo.id()) {
                    aged_tree.root()
                } else {
                    println!("Note not found in aged commitments, using zero root");
                    Fr::from(0u8)
                },
                if latest_tree.contains(&utxo.id()) {
                    latest_tree.root()
                } else {
                    println!("Note not found in latest commitments, using zero root");
                    Fr::from(0u8)
                },
                ledger_state.epoch_state.nonce,
                slot.into(),
                ledger_state.epoch_state.total_stake,
            ),
            leader_key: ed25519_dalek::VerifyingKey::from_bytes(&[0u8; 32]).unwrap(),
            voucher_cm: VoucherCm::default(),
        }
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
            service_params: nomos_core::sdp::ServiceParameters {
                lock_period: 10,
                inactivity_period: 20,
                retention_period: 100,
                timestamp: 0,
            },
            min_stake: nomos_core::sdp::MinStake {
                threshold: 1,
                timestamp: 0,
            },
        }
    }

    #[must_use]
    pub fn genesis_state(utxos: &[Utxo]) -> LedgerState {
        let total_stake = utxos.iter().map(|u| u.note.value).sum();
        let utxos = utxos
            .iter()
            .map(|utxo| (utxo.id(), *utxo))
            .collect::<UtxoTree>();
        LedgerState {
            utxos: utxos.clone(),
            nonce: Fr::ZERO,
            slot: 0.into(),
            next_epoch_state: EpochState {
                epoch: 1.into(),
                nonce: Fr::ZERO,
                utxos: utxos.clone(),
                total_stake,
            },
            epoch_state: EpochState {
                epoch: 0.into(),
                nonce: Fr::ZERO,
                utxos,
                total_stake,
            },
        }
    }

    fn full_ledger_state(cryptarchia_ledger: LedgerState) -> crate::LedgerState {
        crate::LedgerState {
            block_number: 0,
            cryptarchia_ledger,
            mantle_ledger: crate::mantle::LedgerState::default(),
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
        block_state.utxos = block_state.utxos.insert(utxo_add.id(), utxo_add).0;
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
        let proof = DummyProof {
            public: LeaderPublic {
                aged_root: Fr::from(0u8), // Invalid aged root
                latest_root: ledger_state.latest_commitments().root(),
                epoch_nonce: ledger_state.epoch_state.nonce,
                slot: slot.into(),
                total_stake: ledger_state.epoch_state.total_stake,
            },
            leader_key: ed25519_dalek::VerifyingKey::from_bytes(&[0u8; 32]).unwrap(),
            voucher_cm: VoucherCm::default(),
        };
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
        let proof = DummyProof {
            public: LeaderPublic {
                aged_root: ledger_state.aged_commitments().root(),
                latest_root: BigUint::from(1u8).into(), // Invalid latest root
                epoch_nonce: ledger_state.epoch_state.nonce,
                slot: slot.into(),
                total_stake: ledger_state.epoch_state.total_stake,
            },
            leader_key: ed25519_dalek::VerifyingKey::from_bytes(&[0u8; 32]).unwrap(),
            voucher_cm: VoucherCm::default(),
        };
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
            ops_proofs: vec![],
            ledger_tx_proof: DummyZkSignature::prove(ZkSignaturePublic {
                pks,
                msg_hash: mantle_tx.hash().into(),
            }),
            mantle_tx,
        }
    }

    #[test]
    fn test_tx_processing_valid_transaction() {
        let input_note = Note::new(11000, Fr::from(BigUint::from(1u8)).into());
        let input_utxo = Utxo {
            tx_hash: Fr::from(BigUint::from(1u8)).into(),
            output_index: 0,
            note: input_note,
        };

        let output_note1 = Note::new(4000, Fr::from(BigUint::from(2u8)).into());
        let output_note2 = Note::new(3000, Fr::from(BigUint::from(3u8)).into());

        let locked_notes = LockedNotes::new();
        let ledger_state = LedgerState::from_utxos([input_utxo]);
        let tx = create_tx(&[&input_utxo], vec![output_note1, output_note2]);

        let _fees = tx.gas_cost::<MainnetGasConstants>();
        let (new_state, balance) = ledger_state
            .try_apply_tx::<(), MainnetGasConstants>(&locked_notes, tx)
            .unwrap();

        assert_eq!(
            balance,
            i128::from(input_note.value - output_note1.value - output_note2.value)
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
        let locked_notes = LockedNotes::new();
        let _fees = tx.gas_cost::<MainnetGasConstants>();
        let (final_state, final_balance) = new_state
            .try_apply_tx::<(), MainnetGasConstants>(&locked_notes, tx)
            .unwrap();
        assert_eq!(
            final_balance,
            i128::from(output_note1.value + output_note2.value)
        );
        assert!(!final_state.utxos.contains(&output_utxo1.id()));
        assert!(!final_state.utxos.contains(&output_utxo2.id()));
    }

    #[test]
    fn test_tx_processing_invalid_input() {
        let input_note = Note::new(1000, Fr::from(BigUint::from(1u8)).into());
        let input_utxo = Utxo {
            tx_hash: Fr::from(BigUint::from(1u8)).into(),
            output_index: 0,
            note: input_note,
        };

        let non_existent_utxo_1 = Utxo {
            tx_hash: Fr::from(BigUint::from(1u8)).into(),
            output_index: 1,
            note: input_note,
        };

        let non_existent_utxo_2 = Utxo {
            tx_hash: Fr::from(BigUint::from(2u8)).into(),
            output_index: 0,
            note: input_note,
        };

        let non_existent_utxo_3 = Utxo {
            tx_hash: Fr::from(BigUint::from(1u8)).into(),
            output_index: 0,
            note: Note::new(999, Fr::from(BigUint::from(1u8)).into()),
        };

        let ledger_state = LedgerState::from_utxos([input_utxo]);

        let invalid_utxos = [
            non_existent_utxo_1,
            non_existent_utxo_2,
            non_existent_utxo_3,
        ];

        let locked_notes = LockedNotes::new();
        for non_existent_utxo in invalid_utxos {
            let tx = create_tx(&[&non_existent_utxo], vec![]);
            let result = ledger_state
                .clone()
                .try_apply_tx::<(), MainnetGasConstants>(&locked_notes, tx);
            assert!(matches!(result, Err(LedgerError::InvalidNote(_))));
        }
    }

    #[test]
    fn test_tx_processing_insufficient_balance() {
        let input_note = Note::new(1, Fr::from(BigUint::from(1u8)).into());
        let input_utxo = Utxo {
            tx_hash: Fr::from(BigUint::from(1u8)).into(),
            output_index: 0,
            note: input_note,
        };

        let output_note = Note::new(1, Fr::from(BigUint::from(2u8)).into());

        let locked_notes = LockedNotes::new();
        let ledger_state = LedgerState::from_utxos([input_utxo]);
        let tx = create_tx(&[&input_utxo], vec![output_note, output_note]);

        let (_, balance) = ledger_state
            .clone()
            .try_apply_tx::<(), MainnetGasConstants>(&locked_notes, tx)
            .unwrap();
        assert_eq!(balance, -1);

        let tx = create_tx(&[&input_utxo], vec![output_note]);
        assert_eq!(
            ledger_state
                .try_apply_tx::<(), MainnetGasConstants>(&locked_notes, tx)
                .unwrap()
                .1,
            0
        );
    }

    #[test]
    fn test_tx_processing_no_outputs() {
        let input_note = Note::new(10000, Fr::from(BigUint::from(1u8)).into());
        let input_utxo = Utxo {
            tx_hash: Fr::from(BigUint::from(1u8)).into(),
            output_index: 0,
            note: input_note,
        };

        let locked_notes = LockedNotes::new();
        let ledger_state = LedgerState::from_utxos([input_utxo]);
        let tx = create_tx(&[&input_utxo], vec![]);

        let _fees = tx.gas_cost::<MainnetGasConstants>();
        let result = ledger_state.try_apply_tx::<(), MainnetGasConstants>(&locked_notes, tx);
        assert!(result.is_ok());

        let (new_state, balance) = result.unwrap();
        assert_eq!(balance, 10000);

        // Verify input was consumed
        assert!(!new_state.utxos.contains(&input_utxo.id()));
    }

    #[test]
    fn test_output_not_zero() {
        let input_utxo = Utxo {
            tx_hash: Fr::from(BigUint::from(1u8)).into(),
            output_index: 0,
            note: Note::new(10000, Fr::from(BigUint::from(1u8)).into()),
        };

        let locked_notes = LockedNotes::new();
        let ledger_state = LedgerState::from_utxos([input_utxo]);
        let tx = create_tx(
            &[&input_utxo],
            vec![Note::new(0, Fr::from(BigUint::from(2u8)).into())],
        );

        let result = ledger_state.try_apply_tx::<(), MainnetGasConstants>(&locked_notes, tx);
        assert!(matches!(result, Err(LedgerError::ZeroValueNote)));
    }
}
