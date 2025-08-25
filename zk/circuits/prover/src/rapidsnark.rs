use crate::{prover::Prover, wrappers::prover_from_contents};

pub struct Rapidsnark;

impl Prover for Rapidsnark {
    type Error = std::io::Error;

    fn generate_proof(
        proving_key: &[u8],
        witness_contents: &[u8],
    ) -> Result<(Vec<u8>, Vec<u8>), Self::Error> {
        prover_from_contents(proving_key, witness_contents)
    }
}
