use cl::{note::NoteWitness, nullifier::NullifierSecret, NoteCommitment};
use cryptarchia_engine::Slot;
use nomos_core::proofs::leader_proof::Risc0LeaderProof;
use nomos_ledger::{EpochState, NoteTree};
use nomos_proof_statements::leadership::{LeaderPrivate, LeaderPublic};
use serde::{Deserialize, Serialize};

/// TODO: this is a temporary solution until we have a proper wallet
/// implementation. Most notably, it can't track when initial notes are spent
/// and moved
#[derive(Clone)]
pub struct Leader {
    // for each block, the indexes in the note tree of the notes we control
    notes: Vec<NoteWitness>,
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
    pub const fn new(
        notes: Vec<NoteWitness>,
        nf_sk: NullifierSecret,
        config: nomos_ledger::Config,
    ) -> Self {
        Self {
            notes,
            nf_sk,
            config,
        }
    }

    pub async fn build_proof_for(
        &self,
        aged_tree: &NoteTree,
        latest_tree: &NoteTree,
        epoch_state: &EpochState,
        slot: Slot,
    ) -> Option<Risc0LeaderProof> {
        fn find(tree: &NoteTree, note_commit: &NoteCommitment) -> Option<usize> {
            tree.commitments().iter().position(|cm| cm == note_commit)
        }

        for note in &self.notes {
            let note_commit = note.commit(self.nf_sk.commit());

            if !matches!(
                (
                    find(aged_tree, &note_commit),
                    find(latest_tree, &note_commit),
                ),
                (Some(_), Some(_))
            ) {
                tracing::warn!("Note not found in either tree: {:?}", note);
                continue;
            }

            let note_id = [1; 32]; // placeholder for note ID, replace after mantle notes format update

            let public_inputs = LeaderPublic::new(
                aged_tree.root(),
                latest_tree.root(),
                [1; 32], // placeholder for entropy
                *epoch_state.nonce(),
                slot.into(),
                self.config.consensus_config.active_slot_coeff,
                epoch_state.total_stake(),
            );

            if public_inputs.check_winning(note.value, note_id, self.nf_sk.0) {
                tracing::debug!(
                    "leader for slot {:?}, {:?}/{:?}",
                    slot,
                    note.value,
                    epoch_state.total_stake()
                );

                let sk = self.nf_sk.0;
                let private_inputs = LeaderPrivate {
                    value: note.value,
                    note_id,
                    sk,
                };
                let res = tokio::task::spawn_blocking(move || {
                    Risc0LeaderProof::prove(
                        public_inputs,
                        &private_inputs,
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
            } else {
                tracing::debug!(
                    "Not a leader for slot {:?}, {:?}/{:?}",
                    slot,
                    note.value,
                    epoch_state.total_stake()
                );
            }
        }

        None
    }

    pub(crate) fn notes(&self) -> &[NoteWitness] {
        &self.notes
    }
}
