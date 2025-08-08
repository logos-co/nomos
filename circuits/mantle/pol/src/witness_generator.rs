pub trait WitnessGenerator {
    /// The error type returned by the witness generator.
    type Error;

    /// Generates a witness from the given inputs.
    ///
    /// # Arguments
    ///
    /// * `inputs` - A string containing the public and private inputs.
    ///
    /// # Returns
    ///
    /// A [`Result`] which contains the witness if successful.
    fn generate_witness(inputs: &str) -> Result<Vec<u8>, Self::Error>;
}
