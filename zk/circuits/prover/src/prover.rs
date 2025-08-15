pub trait Prover {
    /// The error type returned by the prover.
    type Error;

    /// Generates a proof from the given circuit and witness contents.
    ///
    /// # Arguments
    ///
    /// * `circuit_contents` - A byte slice containing the circuit (proving
    ///   key).
    /// * `witness_contents` - A byte slice containing the witness.
    ///
    /// # Returns
    ///
    /// A [`Result`] which contains the proof and public inputs as strings if
    /// successful,
    fn generate_proof(
        circuit_contents: &[u8],
        witness_contents: &[u8],
    ) -> Result<(Vec<u8>, Vec<u8>), Self::Error>;
}
