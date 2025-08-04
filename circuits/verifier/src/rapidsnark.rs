use crate::{traits::Verifier, verifier_from_contents};

pub struct Rapidsnark;

impl Verifier for Rapidsnark {
    fn verify(
        verification_key_contents: &[u8],
        public_contents: &[u8],
        proof_contents: &[u8],
    ) -> std::io::Result<bool> {
        verifier_from_contents(verification_key_contents, public_contents, proof_contents)
    }
}
