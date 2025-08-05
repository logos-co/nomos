use crate::{Result, prover::Prover, wrappers::prover_from_contents};

pub struct Rapidsnark;

impl Prover for Rapidsnark {
    fn generate_proof(
        circuit_contents: &[u8],
        witness_contents: &[u8],
    ) -> Result<(Vec<u8>, Vec<u8>)> {
        prover_from_contents(circuit_contents, witness_contents)
    }
}
