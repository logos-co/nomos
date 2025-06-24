mod config;
mod crypto;
pub mod leader_proof;
mod notetree;

use std::{collections::HashMap, hash::Hash};

use blake2::Digest as _;
use cl::{balance::Value, note::NoteCommitment, nullifier::Nullifier};
pub use config::Config;
use cryptarchia_engine::{Epoch, Slot};
use crypto::Blake2b;
use nomos_proof_statements::leadership::LeaderPublic;
pub use notetree::NoteTree;
use rpds::{HashTrieSet, HashTrieSetSync};
use thiserror::Error;

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum LedgerError<Id> {
    #[error("Nullifier already exists in the ledger state")]
    DoubleSpend(Nullifier),
    #[error("Invalid block slot {block:?} for parent slot {parent:?}")]
    InvalidSlot { parent: Slot, block: Slot },
    #[error("Parent block not found: {0:?}")]
    ParentNotFound(Id),
    #[error("Invalid leader proof")]
    InvalidProof,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EpochState {
    // The epoch this snapshot is for
    epoch: Epoch,
    // value of the ledger nonce after 'epoch_period_nonce_buffer' slots from the beginning of the
    // epoch
    nonce: [u8; 32],
    // stake distribution snapshot taken at the beginning of the epoch
    // (in practice, this is equivalent to the notes the are spendable at the beginning of the
    // epoch)
    commitments: NoteTree,
    total_stake: Value,
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
        let commitments = if ledger.slot < stake_snapshot_slot {
            ledger.commitments.clone()
        } else {
            self.commitments
        };
        Self {
            epoch: self.epoch,
            nonce,
            commitments,
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

#[derive(Clone, Debug, PartialEq)]
pub struct Ledger<Id: Eq + Hash> {
    states: HashMap<Id, LedgerState>,
    config: Config,
}

impl<Id> Ledger<Id>
where
    Id: Eq + Hash + Copy,
{
    pub fn new(id: Id, state: LedgerState, config: Config) -> Self {
        Self {
            states: std::iter::once((id, state)).collect(),
            config,
        }
    }

    /// Create a new [`Ledger`] instance with the updated state.
    /// This method adds a [`LedgerState`] to the new [`Ledger`] without running
    /// validation.
    ///
    /// # Warning
    ///
    /// **This method bypasses safety checks** and can corrupt the state if used
    /// incorrectly.
    /// Only use for recovery, debugging, or other manipulations where the input
    /// is known to be valid.
    ///
    /// # Arguments
    ///
    /// * `id` - The id of the [`LedgerState`].
    /// * `state` - The [`LedgerState`] to be added.
    #[must_use = "Returns a new instance with the updated state, without modifying the original."]
    pub fn apply_state_unchecked(&self, id: Id, state: LedgerState) -> Self {
        let mut states = self.states.clone();
        states.insert(id, state);
        Self {
            states,
            config: self.config,
        }
    }

    /// Create a new [`Ledger`] with the updated state.
    #[must_use = "Returns a new instance with the updated state, without modifying the original."]
    pub fn try_update<LeaderProof>(
        &self,
        id: Id,
        parent_id: Id,
        slot: Slot,
        proof: &LeaderProof,
    ) -> Result<Self, LedgerError<Id>>
    where
        LeaderProof: leader_proof::LeaderProof,
    {
        let parent_state = self
            .states
            .get(&parent_id)
            .ok_or(LedgerError::ParentNotFound(parent_id))?;

        let new_state = parent_state.clone().try_update(slot, proof, &self.config)?;

        Ok(self.apply_state_unchecked(id, new_state))
    }

    pub fn state(&self, id: &Id) -> Option<&LedgerState> {
        self.states.get(id)
    }

    #[must_use]
    pub const fn config(&self) -> &Config {
        &self.config
    }

    /// Removes the state stored for the given block id.
    ///
    /// This function must be called only when the states being pruned won't be
    /// needed for any subsequent proof.
    ///
    /// ## Arguments
    ///
    /// The block ID to prune the state for.
    ///
    /// ## Returns
    ///
    /// `true` if the state was successfully removed, `false` otherwise.
    pub fn prune_state_at(&mut self, block: &Id) -> bool {
        self.states.remove(block).is_some()
    }
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Eq, PartialEq)]
pub struct LedgerState {
    // commitments to notes that can be spent
    commitments: NoteTree,
    nullifiers: HashTrieSetSync<Nullifier>,
    // randomness contribution
    nonce: [u8; 32],
    slot: Slot,
    // rolling snapshot of the state for the next epoch, used for epoch transitions
    next_epoch_state: EpochState,
    epoch_state: EpochState,
}

impl LedgerState {
    fn try_update<LeaderProof, Id>(
        self,
        slot: Slot,
        proof: &LeaderProof,
        config: &Config,
    ) -> Result<Self, LedgerError<Id>>
    where
        LeaderProof: leader_proof::LeaderProof,
    {
        self.update_epoch_state(slot, config)?
            .try_apply_leadership(slot, proof, config)
    }

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
                commitments: self.commitments.clone(),
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
                commitments: self.commitments.clone(),
                total_stake,
            };
            let next_epoch_state = EpochState {
                epoch: new_epoch + 1,
                nonce: self.nonce,
                commitments: self.commitments.clone(),
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

    fn try_apply_leadership<LeaderProof, Id>(
        mut self,
        slot: Slot,
        proof: &LeaderProof,
        config: &Config,
    ) -> Result<Self, LedgerError<Id>>
    where
        LeaderProof: leader_proof::LeaderProof,
    {
        self = self
            .try_apply_proof(slot, proof, config)?
            .update_nonce(proof.entropy(), slot);

        Ok(self)
    }

    #[must_use]
    pub fn is_nullified(&self, nullifier: &Nullifier) -> bool {
        self.nullifiers.contains(nullifier)
    }

    fn update_nonce(self, contrib: [u8; 32], slot: Slot) -> Self {
        Self {
            nonce: <[u8; 32]>::from(
                Blake2b::new_with_prefix(b"EPOCH_NONCE")
                    .chain_update(self.nonce)
                    .chain_update(contrib)
                    .chain_update(slot.to_be_bytes())
                    .finalize(),
            ),
            ..self
        }
    }

    pub fn from_commitments(
        commitments: impl IntoIterator<Item = NoteCommitment>,
        total_stake: Value,
    ) -> Self {
        let commitments = commitments.into_iter().collect::<NoteTree>();
        Self {
            commitments: commitments.clone(),
            nullifiers: HashTrieSet::default(),
            nonce: [0; 32],
            slot: 0.into(),
            next_epoch_state: EpochState {
                epoch: 1.into(),
                nonce: [0; 32],
                commitments: NoteTree::default(),
                total_stake,
            },
            epoch_state: EpochState {
                epoch: 0.into(),
                nonce: [0; 32],
                commitments,
                total_stake,
            },
        }
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
    pub const fn latest_commitments(&self) -> &NoteTree {
        &self.commitments
    }

    #[must_use]
    pub const fn aged_commitments(&self) -> &NoteTree {
        &self.epoch_state.commitments
    }
}

#[expect(
    clippy::missing_fields_in_debug,
    reason = "No epoch info in debug output."
)]
impl core::fmt::Debug for LedgerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LedgerState")
            .field("commitments", &self.commitments.root())
            .field("nullifiers", &self.nullifiers.iter().collect::<Vec<_>>())
            .field("nonce", &self.nonce)
            .field("slot", &self.slot)
            .finish()
    }
}

#[cfg(test)]
pub mod tests {
    use std::num::NonZero;

    use blake2::Digest as _;
    use cl::{note::NoteWitness as Note, NullifierSecret};
    use cryptarchia_engine::EpochConfig;
    use crypto_bigint::U256;
    use rand::thread_rng;

    use super::*;
    use crate::leader_proof::LeaderProof;

    type HeaderId = [u8; 32];

    const NF_SK: NullifierSecret = NullifierSecret([0; 16]);

    fn note() -> Note {
        Note::basic(0, [0; 32], &mut thread_rng())
    }

    type DummyProof = LeaderPublic;

    impl LeaderProof for DummyProof {
        fn verify(&self, public_inputs: &LeaderPublic) -> bool {
            self == public_inputs
        }

        fn entropy(&self) -> [u8; 32] {
            self.entropy
        }
    }

    fn commit(note: Note) -> NoteCommitment {
        note.commit(NF_SK.commit())
    }

    fn evolve(note: Note) -> Note {
        Note {
            nonce: note.evolved_nonce(NF_SK, b"test"),
            ..note
        }
    }

    fn update_ledger(
        ledger: &mut Ledger<HeaderId>,
        parent: HeaderId,
        slot: impl Into<Slot>,
        note: Note,
    ) -> Result<HeaderId, LedgerError<HeaderId>> {
        let slot = slot.into();
        let ledger_state = ledger
            .state(&parent)
            .unwrap()
            .clone()
            .update_epoch_state::<HeaderId>(slot, ledger.config())
            .unwrap();
        let config = ledger.config();
        let id = make_id(parent, slot, note);
        let proof = generate_proof(&ledger_state, note, slot, config);
        *ledger = ledger.try_update(id, parent, slot, &proof)?;
        Ok(id)
    }

    fn make_id(parent: HeaderId, slot: impl Into<Slot>, note: Note) -> HeaderId {
        Blake2b::new()
            .chain_update(parent)
            .chain_update(slot.into().to_be_bytes())
            .chain_update(commit(note).as_bytes())
            .finalize()
            .into()
    }

    // produce a proof for a note
    fn generate_proof(
        ledger_state: &LedgerState,
        note: Note,
        slot: Slot,
        config: &Config,
    ) -> DummyProof {
        // inefficient implementation, but it's just a test
        fn contains(note_tree: &NoteTree, note: &Note) -> bool {
            note_tree.commitments().iter().any(|n| n == &commit(*note))
        }

        let latest_tree = &ledger_state.latest_commitments();
        let aged_tree = &ledger_state.aged_commitments();

        DummyProof::new(
            if contains(aged_tree, &note) {
                aged_tree.root()
            } else {
                println!("Note not found in latest commitments, using zero root");
                [0; 32]
            },
            if contains(latest_tree, &note) {
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
        )
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
    pub fn genesis_state(commitments: &[NoteCommitment]) -> LedgerState {
        LedgerState {
            commitments: commitments.iter().copied().collect(),
            nullifiers: HashTrieSet::default(),
            nonce: [0; 32],
            slot: 0.into(),
            next_epoch_state: EpochState {
                epoch: 1.into(),
                nonce: [0; 32],
                commitments: commitments.iter().copied().collect(),
                total_stake: 1,
            },
            epoch_state: EpochState {
                epoch: 0.into(),
                nonce: [0; 32],
                commitments: commitments.iter().copied().collect(),
                total_stake: 1,
            },
        }
    }

    fn ledger(commitments: &[NoteCommitment]) -> (Ledger<HeaderId>, HeaderId) {
        let genesis_state = genesis_state(commitments);
        (Ledger::new([0; 32], genesis_state, config()), [0; 32])
    }

    fn apply_and_add_note(
        ledger: &mut Ledger<HeaderId>,
        parent: HeaderId,
        slot: impl Into<Slot>,
        note_proof: Note,
        note_add: Note,
    ) -> HeaderId {
        let id = update_ledger(ledger, parent, slot, note_proof).unwrap();
        // we still don't have transactions, so the only way to add a commitment to
        // spendable commitments and test epoch snapshotting is by doing this
        // manually
        let mut block_state = ledger.states[&id].clone();
        block_state.commitments = block_state.commitments.insert(commit(note_add));
        ledger.states.insert(id, block_state);
        id
    }

    #[test]
    fn test_ledger_state_allow_leadership_note_reuse() {
        let note = note();
        let (mut ledger, genesis) = ledger(&[commit(note)]);

        let h = update_ledger(&mut ledger, genesis, 1, note).unwrap();

        // reusing the same note for leadersip should be prevented
        update_ledger(&mut ledger, h, 2, note).unwrap();
    }

    #[test]
    fn test_ledger_state_uncommited_note() {
        let note = note();
        let (mut ledger, genesis) = ledger(&[]);
        assert!(matches!(
            update_ledger(&mut ledger, genesis, 1, note),
            Err(LedgerError::InvalidProof),
        ));
    }

    #[test]
    fn test_ledger_state_is_properly_updated_on_reorg() {
        let note_1 = note();
        let note_2 = note();
        let note_3 = note();

        let (mut ledger, genesis) = ledger(&[commit(note_1), commit(note_2), commit(note_3)]);

        // note_1 & note_2 both concurrently win slot 0

        update_ledger(&mut ledger, genesis, 1, note_1).unwrap();
        let h = update_ledger(&mut ledger, genesis, 1, note_2).unwrap();

        // then note_3 wins slot 1 and chooses to extend from block_2
        let h_3 = update_ledger(&mut ledger, h, 2, note_3).unwrap();
        // note 1 is not spent in the chain that ends with block_3
        assert!(!ledger.states[&h_3].is_nullified(&Nullifier::new(NF_SK, commit(note_1))));
    }

    #[test]
    fn test_epoch_transition() {
        let notes = std::iter::repeat_with(note).take(4).collect::<Vec<_>>();
        let note_4 = note();
        let note_5 = note();
        let (mut ledger, genesis) = ledger(&notes.iter().copied().map(commit).collect::<Vec<_>>());

        // An epoch will be 10 slots long, with stake distribution snapshot taken at the
        // start of the epoch and nonce snapshot before slot 7

        let h_1 = update_ledger(&mut ledger, genesis, 1, notes[0]).unwrap();
        assert_eq!(ledger.states[&h_1].epoch_state.epoch, 0.into());

        let h_2 = update_ledger(&mut ledger, h_1, 6, notes[1]).unwrap();

        let h_3 = apply_and_add_note(&mut ledger, h_2, 9, notes[2], note_4);

        // test epoch jump
        let h_4 = update_ledger(&mut ledger, h_3, 20, notes[3]).unwrap();
        // nonce for epoch 2 should be taken at the end of slot 16, but in our case the
        // last block is at slot 9
        assert_eq!(
            ledger.states[&h_4].epoch_state.nonce,
            ledger.states[&h_3].nonce,
        );
        // stake distribution snapshot should be taken at the end of slot 9
        assert_eq!(
            ledger.states[&h_4].epoch_state.commitments,
            ledger.states[&h_3].commitments,
        );

        // nonce for epoch 1 should be taken at the end of slot 6
        update_ledger(&mut ledger, h_3, 10, notes[3]).unwrap();
        let h_5 = apply_and_add_note(&mut ledger, h_3, 10, notes[3], note_5);
        assert_eq!(
            ledger.states[&h_5].epoch_state.nonce,
            ledger.states[&h_2].nonce,
        );

        let h_6 = update_ledger(&mut ledger, h_5, 20, notes[3]).unwrap();
        // stake distribution snapshot should be taken at the end of slot 9, check that
        // changes in slot 10 are ignored
        assert_eq!(
            ledger.states[&h_6].epoch_state.commitments,
            ledger.states[&h_3].commitments,
        );
    }

    #[test]
    fn test_no_leadership_note_evolution() {
        let note = note();
        let (mut ledger, genesis) = ledger(&[commit(note)]);

        let h = update_ledger(&mut ledger, genesis, 1, note).unwrap();

        // reusing the same note should be allow, as PoLv2 does not consume a note
        update_ledger(&mut ledger, h, 2, note).unwrap();

        // no evolved note is not eligible for leadership
        assert!(matches!(
            update_ledger(&mut ledger, genesis, 2, evolve(note)),
            Err(LedgerError::InvalidProof),
        ));
        // the evolved note is eligible after note 1 is spent
        assert!(matches!(
            update_ledger(&mut ledger, h, 2, evolve(note)),
            Err(LedgerError::InvalidProof),
        ));
    }

    #[test]
    fn test_new_notes_becoming_eligible_after_stake_distribution_stabilizes() {
        let note_1 = note();
        let note = note();

        let (mut ledger, genesis) = ledger(&[commit(note)]);

        // EPOCH 0
        // mint a new note to be used for leader elections in upcoming epochs
        let h_0_1 = apply_and_add_note(&mut ledger, genesis, 1, note, note_1);

        // the new note is not yet eligible for leader elections
        assert!(matches!(
            update_ledger(&mut ledger, h_0_1, 2, note_1),
            Err(LedgerError::InvalidProof),
        ));

        // EPOCH 1
        for i in 10..20 {
            // the newly minted note is still not eligible in the following epoch since the
            // stake distribution snapshot is taken at the beginning of the previous epoch
            assert!(matches!(
                update_ledger(&mut ledger, h_0_1, i, note_1),
                Err(LedgerError::InvalidProof),
            ));
        }

        // EPOCH 2
        // the note is finally eligible 2 epochs after it was first minted
        update_ledger(&mut ledger, h_0_1, 20, note_1).unwrap();
    }

    #[test]
    fn test_update_epoch_state_with_outdated_slot_error() {
        let note = note();
        let commitment = commit(note);
        let (ledger, genesis) = ledger(&[commitment]);

        let ledger_state = ledger.state(&genesis).unwrap().clone();
        let ledger_config = ledger.config();

        let slot = Slot::genesis() + 10;
        let ledger_state2 = ledger_state
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
        let note = note();
        let commitment = commit(note);
        let (ledger, genesis) = ledger(&[commitment]);
        let ledger_state = ledger.state(&genesis).unwrap().clone();
        let slot = Slot::genesis() + 1;
        let proof = DummyProof {
            aged_root: [1; 32], // Invalid aged root
            latest_root: ledger_state.commitments.root(),
            epoch_nonce: ledger_state.epoch_state.nonce,
            slot: slot.into(),
            entropy: [1; 32],
            scaled_phi_approx: (U256::from(1u32), U256::from(1u32)),
        };
        let update_err = ledger_state
            .try_apply_proof::<_, ()>(slot, &proof, ledger.config())
            .err();

        assert_eq!(Some(LedgerError::InvalidProof), update_err);
    }

    #[test]
    fn test_invalid_latest_root_rejected() {
        let note = note();
        let commitment = commit(note);
        let (ledger, genesis) = ledger(&[commitment]);
        let ledger_state = ledger.state(&genesis).unwrap().clone();
        let slot = Slot::genesis() + 1;
        let proof = DummyProof {
            aged_root: [1; 32], // Invalid aged root
            latest_root: ledger_state.commitments.root(),
            epoch_nonce: ledger_state.epoch_state.nonce,
            slot: slot.into(),
            entropy: [1; 32],
            scaled_phi_approx: (U256::from(1u32), U256::from(1u32)),
        };
        let update_err = ledger_state
            .try_apply_proof::<_, ()>(slot, &proof, ledger.config())
            .err();

        assert_eq!(Some(LedgerError::InvalidProof), update_err);
    }
}
