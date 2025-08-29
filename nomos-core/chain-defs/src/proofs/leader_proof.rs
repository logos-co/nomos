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
