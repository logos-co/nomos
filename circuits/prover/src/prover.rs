use std::io;

pub trait Prover {
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
    /// An [`io::Result<(String, String)>`] which contains the proof and public
    /// inputs as strings if successful, or an [`io::Error`] if the command
    /// fails.
    fn generate_proof(
        circuit_contents: &[u8],
        witness_contents: &[u8],
    ) -> io::Result<(String, String)>;
}
