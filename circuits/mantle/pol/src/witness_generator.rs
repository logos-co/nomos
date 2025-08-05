use std::io;

pub trait WitnessGenerator {
    /// Generates a witness from the given inputs.
    ///
    /// # Arguments
    ///
    /// * `inputs` - A string containing the public and private inputs.
    ///
    /// # Returns
    ///
    /// An [`io::Result<Vec<u8>>`] which contains the witness if successful, or
    /// an [`io::Error`] if the command fails.
    fn generate_witness(inputs: &str) -> io::Result<Vec<u8>>;
}
