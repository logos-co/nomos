mod config;
mod crypto;
pub mod leader_proof;
mod notetree;

use std::{collections::HashMap, hash::Hash};

use blake2::Digest;
use cl::{balance::Value, note::NoteCommitment, nullifier::Nullifier};
pub use config::Config;
use cryptarchia_engine::{Epoch, Slot};
use crypto::Blake2b;
use leader_proof::OrphanProof;
use nomos_proof_statements::leadership::LeaderPublic;
pub use notetree::NoteTree;
use rpds::{HashTrieSet, HashTrieSetSync};
use thiserror::Error;

#[derive(Clone, Debug, Error)]
pub enum LedgerError<Id> {
    #[error("Nullifier already exists in the ledger state")]
    DoubleSpend(Nullifier),
    #[error("Invalid block slot {block:?} for parent slot {parent:?}")]
    InvalidSlot { parent: Slot, block: Slot },
    #[error("Parent block not found: {0:?}")]
    ParentNotFound(Id),
    #[error("Orphan block missing: {0:?}. Importing leader proofs requires the block to be validated first")]
    OrphanMissing(Id),
    #[error("Invalid leader proof")]
    InvalidProof,
    #[error("Invalid leader proof root")]
    InvalidRoot,
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
            ledger.spend_commitments.clone()
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

    #[must_use = "this returns the result of the operation, without modifying the original"]
    pub fn try_update<LeaderProof>(
        &self,
        id: Id,
        parent_id: Id,
        slot: Slot,
        proof: &LeaderProof,
        // (update corresponding to the leader proof, leader proof)
        orphan_proofs: impl IntoIterator<Item = (Id, OrphanProof)>,
    ) -> Result<Self, LedgerError<Id>>
    where
        LeaderProof: leader_proof::LeaderProof,
    {
        let parent_state = self
            .states
            .get(&parent_id)
            .ok_or(LedgerError::ParentNotFound(parent_id))?;

        // TODO: remove this extra logic, we can simply check the proof is valid and the
        // root is a valid one just like we do anyway
        // Oprhan proofs need to be:
        // * locally valid for the block they were originally in
        // * not in conflict with the current ledger state
        // This first condition is checked here, the second one is checked in the state
        // update (in particular, we do not check the imported leader proof is
        // for an earlier slot)
        let (orphan_ids, orphan_proofs): (Vec<_>, Vec<_>) = orphan_proofs.into_iter().unzip();
        for orphan_id in orphan_ids {
            if !self.states.contains_key(&orphan_id) {
                return Err(LedgerError::OrphanMissing(orphan_id));
            }
        }

        let new_state =
            parent_state
                .clone()
                .try_update(slot, proof, &orphan_proofs, &self.config)?;

        let mut states = self.states.clone();

        states.insert(id, new_state);

        Ok(Self {
            states,
            config: self.config,
        })
    }

    pub fn state(&self, id: &Id) -> Option<&LedgerState> {
        self.states.get(id)
    }

    #[must_use]
    pub const fn config(&self) -> &Config {
        &self.config
    }
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Eq, PartialEq)]
pub struct LedgerState {
    // commitments to notes that can be used to propose new blocks
    lead_commitments: NoteTree,
    // commitments to notes that can be spent, this is a superset of lead_commitments
    spend_commitments: NoteTree,
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
        orphan_proofs: &[OrphanProof],
        config: &Config,
    ) -> Result<Self, LedgerError<Id>>
    where
        LeaderProof: leader_proof::LeaderProof,
    {
        self.update_epoch_state(slot, config)?.try_apply_leadership(
            slot,
            proof,
            orphan_proofs,
            config,
        )
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
                commitments: self.spend_commitments.clone(),
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
                commitments: self.spend_commitments.clone(),
                total_stake,
            };
            let next_epoch_state = EpochState {
                epoch: new_epoch + 1,
                nonce: self.nonce,
                commitments: self.spend_commitments.clone(),
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

    fn try_apply_proof_outputs<Id>(
        self,
        cm_root: [u8; 32],
        nullifier: Nullifier,
        evolved_commitment: NoteCommitment,
    ) -> Result<Self, LedgerError<Id>> {
        // The leadership note either has to be in the state snapshot or be derived from
        // a note that is in the state snapshot (i.e. be in the lead coins commitments)
        if !self.lead_commitments.is_valid_root(&cm_root)
            && !self.epoch_state.commitments.is_valid_root(&cm_root)
        {
            return Err(LedgerError::InvalidRoot);
        }

        if self.is_nullified(&nullifier) {
            return Err(LedgerError::DoubleSpend(nullifier));
        }

        let lead_commitments = self.lead_commitments.insert(evolved_commitment);
        let spend_commitments = self.spend_commitments.insert(evolved_commitment);
        let nullifiers = self.nullifiers.insert(nullifier);

        Ok(Self {
            lead_commitments,
            spend_commitments,
            nullifiers,
            ..self
        })
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
            proof.merke_root(),
            self.epoch_state.nonce,
            slot.into(),
            config.consensus_config.active_slot_coeff,
            self.epoch_state.total_stake,
            proof.nullifier(),
            proof.evolved_commitment(),
        );
        if !proof.verify(&public_inputs) {
            return Err(LedgerError::InvalidProof);
        }

        self.try_apply_proof_outputs(
            proof.merke_root(),
            proof.nullifier(),
            proof.evolved_commitment(),
        )
    }

    fn try_apply_leadership<LeaderProof, Id>(
        mut self,
        slot: Slot,
        proof: &LeaderProof,
        orphan_proofs: &[OrphanProof],
        config: &Config,
    ) -> Result<Self, LedgerError<Id>>
    where
        LeaderProof: leader_proof::LeaderProof,
    {
        for OrphanProof {
            nullifier,
            commitment,
            cm_root,
        } in orphan_proofs
        {
            self = self.try_apply_proof_outputs(*cm_root, *nullifier, *commitment)?;
        }

        self = self
            .try_apply_proof(slot, proof, config)?
            .update_nonce(proof.nullifier(), slot);

        Ok(self)
    }

    #[must_use]
    pub fn is_nullified(&self, nullifier: &Nullifier) -> bool {
        self.nullifiers.contains(nullifier)
    }

    fn update_nonce(self, nullifier: Nullifier, slot: Slot) -> Self {
        Self {
            nonce: <[u8; 32]>::from(
                Blake2b::new_with_prefix(b"epoch-nonce")
                    .chain_update(self.nonce)
                    .chain_update(nullifier.as_bytes())
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
            lead_commitments: commitments.clone(),
            spend_commitments: commitments,
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
                commitments: NoteTree::default(),
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
    pub const fn lead_commitments(&self) -> &NoteTree {
        &self.lead_commitments
    }
}

#[expect(
    clippy::missing_fields_in_debug,
    reason = "No epoch info in debug output."
)]
impl core::fmt::Debug for LedgerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LedgerState")
            .field("lead_commitment", &self.lead_commitments.root())
            .field("spend_commitments", &self.spend_commitments.root())
            .field("nullifiers", &self.nullifiers.iter().collect::<Vec<_>>())
            .field("nonce", &self.nonce)
            .field("slot", &self.slot)
            .finish()
    }
}

#[cfg(test)]
pub mod tests {
    use std::num::NonZero;

    use blake2::Digest;
    use cl::{note::NoteWitness as Note, NullifierSecret};
    use cryptarchia_engine::{EpochConfig, Slot};
    use rand::thread_rng;
    use rpds::HashTrieSet;

    use super::*;
    use crate::{crypto::Blake2b, leader_proof::LeaderProof, Config, LedgerError};

    type HeaderId = [u8; 32];

    const NF_SK: NullifierSecret = NullifierSecret([0; 16]);

    fn note() -> Note {
        Note::basic(0, [0; 32], &mut thread_rng())
    }

    struct DummyProof {
        cm_root: [u8; 32],
        nullifier: Nullifier,
        commitment: NoteCommitment,
    }

    impl LeaderProof for DummyProof {
        fn nullifier(&self) -> Nullifier {
            self.nullifier
        }

        fn evolved_commitment(&self) -> NoteCommitment {
            self.commitment
        }

        fn verify(&self, public_inputs: &LeaderPublic) -> bool {
            self.cm_root == public_inputs.cm_root
        }

        fn merke_root(&self) -> [u8; 32] {
            self.cm_root
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
        update_orphans(ledger, parent, slot, note, vec![])
    }

    fn make_id(parent: HeaderId, slot: impl Into<Slot>, note: Note) -> HeaderId {
        Blake2b::new()
            .chain_update(parent)
            .chain_update(slot.into().to_be_bytes())
            .chain_update(commit(note).as_bytes())
            .finalize()
            .into()
    }

    // produce a proof for a note and included orphan proofs
    fn generate_proofs(
        ledger_state: &LedgerState,
        note: Note,
        orphans: Vec<(HeaderId, Note)>,
    ) -> (DummyProof, Vec<(HeaderId, OrphanProof)>) {
        // inefficient implementation, but it's just a test
        fn contains(note_tree: &NoteTree, note: &Note) -> bool {
            note_tree.commitments().iter().any(|n| n == &commit(*note))
        }

        fn proof(note: Note, cm_root: [u8; 32]) -> DummyProof {
            DummyProof {
                cm_root,
                nullifier: Nullifier::new(NF_SK, commit(note)),
                commitment: commit(evolve(note)),
            }
        }

        fn get_cm_root(note: Note, trees: &[&NoteTree]) -> [u8; 32] {
            for tree in trees {
                if contains(tree, &note) {
                    return tree.root();
                }
            }
            [0; 32]
        }

        // for each proof, the root used is either the lead commitment root or the
        // snapshot root if they contain the note, or a [0; 32] root if the node
        // is not among the allowed commitments Note that the lead commitment
        // root is evolved as the orphan proofs are produce, just like a node would
        // update it after processing orphan proofs
        let mut lead_comms = ledger_state.lead_commitments().clone();
        let snapshot_comms = &ledger_state.epoch_state().commitments;
        let (ids, imported_note): (Vec<_>, Vec<_>) = orphans.into_iter().unzip();
        let orphans = imported_note
            .into_iter()
            .map(|note| {
                let cm_root = get_cm_root(note, &[&lead_comms, snapshot_comms]);
                lead_comms = lead_comms.insert(evolve(note).commit(NF_SK.commit()));
                proof(note, cm_root).to_orphan_proof()
            })
            .zip(ids)
            .map(|(orphan, id)| (id, orphan))
            .collect::<Vec<_>>();

        let cm_root = get_cm_root(note, &[&lead_comms, snapshot_comms]);
        let proof = proof(note, cm_root);

        (proof, orphans)
    }

    fn update_orphans(
        ledger: &mut Ledger<HeaderId>,
        parent: HeaderId,
        slot: impl Into<Slot>,
        note: Note,
        orphans: Vec<(HeaderId, Note)>,
    ) -> Result<HeaderId, LedgerError<HeaderId>> {
        let slot = slot.into();
        let ledger_state = ledger
            .state(&parent)
            .unwrap()
            .clone()
            .update_epoch_state::<HeaderId>(slot, ledger.config())
            .unwrap();
        let id = make_id(parent, slot, note);
        let (proof, orphan_proofs) = generate_proofs(&ledger_state, note, orphans);
        *ledger = ledger.try_update(id, parent, slot, &proof, orphan_proofs)?;
        Ok(id)
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
            lead_commitments: commitments.iter().copied().collect(),
            spend_commitments: commitments.iter().copied().collect(),
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
        block_state.spend_commitments = block_state.spend_commitments.insert(commit(note_add));
        ledger.states.insert(id, block_state);
        id
    }

    #[test]
    fn test_ledger_state_prevents_note_reuse() {
        let note = note();
        let (mut ledger, genesis) = ledger(&[commit(note)]);

        let h = update_ledger(&mut ledger, genesis, 1, note).unwrap();

        // reusing the same note should be prevented
        assert!(matches!(
            update_ledger(&mut ledger, h, 2, note),
            Err(LedgerError::DoubleSpend(_)),
        ));
    }

    #[test]
    fn test_ledger_state_uncommited_note() {
        let note = note();
        let (mut ledger, genesis) = ledger(&[]);
        assert!(matches!(
            update_ledger(&mut ledger, genesis, 1, note),
            Err(LedgerError::InvalidRoot),
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
        let notes = (0..4).map(|_| note()).collect::<Vec<_>>();
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
            ledger.states[&h_3].spend_commitments,
        );

        // nonce for epoch 1 should be taken at the end of slot 6
        update_ledger(&mut ledger, h_3, 10, notes[3]).unwrap();
        let h_5 = apply_and_add_note(&mut ledger, h_3, 10, notes[3], note_5);
        assert_eq!(
            ledger.states[&h_5].epoch_state.nonce,
            ledger.states[&h_2].nonce,
        );

        let h_6 = update_ledger(&mut ledger, h_5, 20, evolve(notes[3])).unwrap();
        // stake distribution snapshot should be taken at the end of slot 9, check that
        // changes in slot 10 are ignored
        assert_eq!(
            ledger.states[&h_6].epoch_state.commitments,
            ledger.states[&h_3].spend_commitments,
        );
    }

    #[test]
    fn test_evolved_note_is_eligible_for_leadership() {
        let note = note();
        let (mut ledger, genesis) = ledger(&[commit(note)]);

        let h = update_ledger(&mut ledger, genesis, 1, note).unwrap();

        // reusing the same note should be prevented
        assert!(matches!(
            update_ledger(&mut ledger, h, 2, note),
            Err(LedgerError::DoubleSpend(_)),
        ));

        // the evolved note is not elibile before block 2 as it has not appeared on the
        // ledger yet
        assert!(matches!(
            update_ledger(&mut ledger, genesis, 2, evolve(note)),
            Err(LedgerError::InvalidRoot),
        ));

        // the evolved note is eligible after note 1 is spent
        assert!(update_ledger(&mut ledger, h, 2, evolve(note)).is_ok());
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
            Err(LedgerError::InvalidRoot),
        ));

        // // but the evolved note can
        let h_0_2 = update_ledger(&mut ledger, h_0_1, 2, evolve(note)).unwrap();

        // EPOCH 1
        for i in 10..20 {
            // the newly minted note is still not eligible in the following epoch since the
            // stake distribution snapshot is taken at the beginning of the previous epoch
            assert!(matches!(
                update_ledger(&mut ledger, h_0_2, i, note_1),
                Err(LedgerError::InvalidRoot),
            ));
        }

        // EPOCH 2
        // the note is finally eligible 2 epochs after it was first minted
        let h_2_0 = update_ledger(&mut ledger, h_0_2, 20, note_1).unwrap();

        // and now the minted note can freely use the evolved note for subsequent blocks
        update_ledger(&mut ledger, h_2_0, 21, evolve(note_1)).unwrap();
    }

    #[test]
    fn test_orphan_proof_import() {
        let note = note();
        let (mut ledger, genesis) = ledger(&[commit(note)]);

        let note_new = evolve(note);
        let note_new_new = evolve(note_new);

        // produce a fork where the note has been spent twice
        let fork_1 = make_id(genesis, 1, note);
        let fork_2 = make_id(fork_1, 2, note_new);

        // neither of the evolved notes should be usable right away in another branch
        assert!(matches!(
            update_ledger(&mut ledger, genesis, 1, note_new),
            Err(LedgerError::InvalidRoot)
        ));
        assert!(matches!(
            update_ledger(&mut ledger, genesis, 1, note_new_new),
            Err(LedgerError::InvalidRoot)
        ));

        // they also should not be accepted if the fork from where they have been
        // imported has not been seen already
        assert!(matches!(
            update_orphans(&mut ledger, genesis, 1, note_new, vec![(fork_1, note)]),
            Err(LedgerError::OrphanMissing(_))
        ));

        // now the first block of the fork is seen (and accepted)
        let h_1 = update_ledger(&mut ledger, genesis, 1, note).unwrap();
        assert_eq!(h_1, fork_1);

        // and it can now be imported in another branch (note this does not validate
        // it's for an earlier slot)
        update_orphans(
            &mut ledger.clone(),
            genesis,
            1,
            note_new,
            vec![(fork_1, note)],
        )
        .unwrap();
        // but the next note is still not accepted since the second block using the
        // evolved note has not been seen yet
        assert!(matches!(
            update_orphans(
                &mut ledger.clone(),
                genesis,
                1,
                note_new_new,
                vec![(fork_1, note), (fork_2, note_new)],
            ),
            Err(LedgerError::OrphanMissing(_))
        ));

        // now the second block of the fork is seen as well and the note evolved twice
        // can be used in another branch
        let h_2 = update_ledger(&mut ledger, h_1, 2, note_new).unwrap();
        assert_eq!(h_2, fork_2);
        update_orphans(
            &mut ledger.clone(),
            genesis,
            1,
            note_new_new,
            vec![(fork_1, note), (fork_2, note_new)],
        )
        .unwrap();
        // but we can't import just the second proof because it's using an evolved note
        // that has not been seen yet
        assert!(matches!(
            update_orphans(
                &mut ledger.clone(),
                genesis,
                1,
                note_new_new,
                vec![(fork_2, note_new)],
            ),
            Err(LedgerError::InvalidRoot)
        ));

        // an imported proof that uses a note that was already used in the base branch
        // should not be allowed
        let header_1 = update_ledger(&mut ledger, genesis, 1, note).unwrap();
        assert!(matches!(
            update_orphans(
                &mut ledger,
                header_1,
                2,
                note_new_new,
                vec![(fork_1, note), (fork_2, note_new)],
            ),
            Err(LedgerError::DoubleSpend(_))
        ));
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
}
