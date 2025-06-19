use nomos_proof_statements::leadership::{LeaderPrivate, LeaderPublic};
use serde::{de::Error as _, Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct Risc0LeaderProof {
    public_inputs: LeaderPublic,
    risc0_receipt: risc0_zkvm::Receipt,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Risc0 proof failed: {0}")]
    Risc0ProofFailed(#[from] anyhow::Error),
}

impl Risc0LeaderProof {
    pub fn prove(
        public_inputs: LeaderPublic,
        private_inputs: &LeaderPrivate,
        prover: &dyn risc0_zkvm::Prover,
    ) -> Result<Self, Error> {
        let env = risc0_zkvm::ExecutorEnv::builder()
            .write(&public_inputs)
            .unwrap()
            .write(&private_inputs)
            .unwrap()
            .build()
            .unwrap();

        let start_t = std::time::Instant::now();

        // ATTENTION: producing a groth16 proof currently requires x86 with docker
        // support
        let opts = risc0_zkvm::ProverOpts::groth16();
        let prove_info =
            prover.prove_with_opts(env, nomos_risc0_proofs::PROOF_OF_LEADERSHIP_ELF, &opts)?;

        tracing::debug!(
            "STARK prover time: {:.2?}, total_cycles: {}",
            start_t.elapsed(),
            prove_info.stats.total_cycles
        );
        // extract the receipt.
        Ok(Self {
            public_inputs,
            risc0_receipt: prove_info.receipt,
        })
    }
}

pub trait LeaderProof {
    /// Verify the proof against the public inputs.
    fn verify(&self, public_inputs: &LeaderPublic) -> bool;

    /// Get the entropy used in the proof.
    fn entropy(&self) -> [u8; 32];
}

impl LeaderProof for Risc0LeaderProof {
    fn verify(&self, public_inputs: &LeaderPublic) -> bool {
        // The risc0 proof is valid by contract
        public_inputs == &self.public_inputs
    }

    fn entropy(&self) -> [u8; 32] {
        self.public_inputs.entropy
    }
}

impl Serialize for Risc0LeaderProof {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.risc0_receipt.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Risc0LeaderProof {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let risc0_receipt = risc0_zkvm::Receipt::deserialize(deserializer)?;
        let public_inputs = risc0_receipt
            .journal
            .decode()
            .map_err(|e| D::Error::custom(format!("Invalid public inputs: {e}")))?;

        risc0_receipt
            .verify(nomos_risc0_proofs::PROOF_OF_LEADERSHIP_ID)
            .map_err(D::Error::custom)?;

        Ok(Self {
            public_inputs,
            risc0_receipt,
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::mantle::{merkle, Note, Utxo};

    const MAX_NOTE_COMMS: usize = 1 << 8;

    fn note_id_leaves(note_ids: &[Utxo]) -> [[u8; 32]; MAX_NOTE_COMMS] {
        let note_comm_bytes: Vec<Vec<u8>> = note_ids.iter().map(|c| c.id().0.to_vec()).collect();
        merkle::padded_leaves::<MAX_NOTE_COMMS>(&note_comm_bytes)
    }

    #[test]
    fn test_leader_prover() {
        let sk = [1; 16];
        let pk = [2; 32]; // TODO: derive from sk, this is not yet checked in the proof
        let note = Note::new(32, pk.into());

        let aged_notes = vec![Utxo {
            tx_hash: [0; 32].into(),
            output_index: 0,
            note,
        }];
        let aged_leaves = note_id_leaves(&aged_notes);

        let latest_notes = aged_notes; // For simplicity, using the same notes for latest
        let latest_leaves = note_id_leaves(&latest_notes);
        // placeholder
        let note_id = [0; 32];

        let epoch_nonce = [0u8; 32];
        let slot = 0;
        let active_slot_coefficient = 0.05;
        let total_stake = 1000;

        let mut expected_public_inputs = LeaderPublic::new(
            merkle::root(aged_leaves),
            merkle::root(latest_leaves),
            [1; 32], // placeholder for entropy
            epoch_nonce,
            slot,
            active_slot_coefficient,
            total_stake,
        );

        while !expected_public_inputs.check_winning(note.value, note_id, sk) {
            expected_public_inputs.slot += 1;
        }

        println!("slot={}", expected_public_inputs.slot);

        let private_inputs = LeaderPrivate {
            value: note.value,
            note_id,
            sk,
        };

        let proof = Risc0LeaderProof::prove(
            expected_public_inputs,
            &private_inputs,
            risc0_zkvm::default_prover().as_ref(),
        )
        .unwrap();
        assert!(proof
            .risc0_receipt
            .verify(nomos_risc0_proofs::PROOF_OF_LEADERSHIP_ID)
            .is_ok());

        assert_eq!(
            expected_public_inputs,
            proof.risc0_receipt.journal.decode().unwrap()
        );
    }
}
