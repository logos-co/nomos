use crate::{traits::Verifier, wrappers::verifier_from_contents};

pub struct Rapidsnark;

impl Verifier for Rapidsnark {
    type Error = std::io::Error;

    fn verify(
        verification_key_contents: &[u8],
        public_contents: &[u8],
        proof_contents: &[u8],
    ) -> Result<bool, Self::Error> {
        verifier_from_contents(verification_key_contents, public_contents, proof_contents)
    }
}
