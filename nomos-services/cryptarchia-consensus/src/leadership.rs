use std::collections::HashMap;

use cl::{
    note::NoteWitness,
    nullifier::{Nullifier, NullifierSecret},
    InputWitness,
};
use cryptarchia_engine::Slot;
use nomos_core::{header::HeaderId, proofs::leader_proof::Risc0LeaderProof};
use nomos_ledger::{EpochState, NoteTree};
use nomos_proof_statements::leadership::{LeaderPrivate, LeaderPublic};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct Leader {
    // for each block, the indexes in the note tree of the notes we control
    notes: HashMap<HeaderId, Vec<NoteWitness>>,
    nf_sk: NullifierSecret,
    config: nomos_ledger::Config,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LeaderConfig {
    pub notes: Vec<NoteWitness>,
    // this is common to every note
    pub nf_sk: NullifierSecret,
}

impl Leader {
    pub fn new(
        header_id: HeaderId,
        header_notes: Vec<NoteWitness>,
        nf_sk: NullifierSecret,
        config: nomos_ledger::Config,
    ) -> Self {
        Self {
            notes: HashMap::from([(header_id, header_notes)]),
            nf_sk,
            config,
        }
    }

    // Signal that the chain extended with a new header, possibly evolving a leader
    // notes in the process FIXME: this implementation does not delete old coins
    // and will attempt to re-use a note in different forks,        we should
    // use the orphan proofs mechanism to handle this.
    pub fn follow_chain(&mut self, parent_id: HeaderId, id: HeaderId, to_evolve: Nullifier) {
        if let Some(notes) = self.notes.get(&parent_id) {
            let notes = notes
                .iter()
                .map(|note| {
                    let note_cm = note.commit(self.nf_sk.commit());
                    if Nullifier::new(self.nf_sk, note_cm) == to_evolve {
                        evolve(note, self.nf_sk)
                    } else {
                        *note
                    }
                })
                .collect();
            self.notes.insert(id, notes);
        }
    }

    pub async fn build_proof_for(
        &self,
        note_tree: &NoteTree,
        epoch_state: &EpochState,
        slot: Slot,
        parent: HeaderId,
    ) -> Option<Risc0LeaderProof> {
        let notes = self.notes.get(&parent)?;
        for note in notes {
            let note_commit = note.commit(self.nf_sk.commit());
            let Some(index) = note_tree
                .commitments()
                .iter()
                .position(|cm| cm == &note_commit)
            else {
                continue;
            };

            let input_cm_path = note_tree
                .witness(index)
                .expect("Note was found in the tree");
            let note = InputWitness {
                note: *note,
                nf_sk: self.nf_sk,
                cm_path: input_cm_path,
            };
            let public_inputs = LeaderPublic::new(
                note_tree.root(),
                *epoch_state.nonce(),
                slot.into(),
                self.config.consensus_config.active_slot_coeff,
                epoch_state.total_stake(),
                Nullifier::new(self.nf_sk, note_commit),
                note.evolve_output(b"NOMOS_POL").commit_note(),
            );
            if public_inputs.check_winning(&note) {
                tracing::debug!(
                    "leader for slot {:?}, {:?}/{:?}",
                    slot,
                    note.note.value,
                    epoch_state.total_stake()
                );
                let input = note.clone();
                let res = tokio::task::spawn_blocking(move || {
                    Risc0LeaderProof::prove(
                        public_inputs,
                        &LeaderPrivate { input },
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
            }
        }

        None
    }

    pub(crate) fn notes(&self, header_id: &HeaderId) -> Option<&[NoteWitness]> {
        self.notes.get(header_id).map(Vec::as_slice)
    }

    /// Removes the notes stored for the given block id.
    ///
    /// This function must be called only when the notes being pruned won't be
    /// needed for any subsequent proof going forward.
    ///
    /// ## Arguments
    ///
    /// The block ID to prune the state for.
    ///
    /// ## Returns
    ///
    /// `true` if the state was successfully removed, `false` otherwise (e.g.,
    /// if no state was associated to the provided block ID).
    pub(crate) fn prune_notes_at(&mut self, header_id: &HeaderId) -> bool {
        self.notes.remove(header_id).is_some()
    }
}

fn evolve(note: &NoteWitness, nf_sk: NullifierSecret) -> NoteWitness {
    NoteWitness {
        nonce: note.evolved_nonce(nf_sk, b"NOMOS_POL"),
        ..*note
    }
}
