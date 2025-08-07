pub trait Verifier {
    /// The error type returned by the prover.
    type Error;

    /// Verifies a proof using the provided verification key and public inputs.
    ///
    /// # Arguments
    ///
    /// * `verification_key_contents` - A byte slice containing the verification
    ///   key.
    /// * `public_contents` - A byte slice containing the public inputs.
    /// * `proof_contents` - A byte slice containing the proof.
    ///
    /// # Returns
    ///
    /// An [`io::Result<bool>`] which indicates whether the verification was
    /// successful or not, or an [`io::Error`] if the command fails.
    fn verify(
        verification_key_contents: &[u8],
        public_contents: &[u8],
        proof_contents: &[u8],
    ) -> Result<bool, Self::Error>;
}
