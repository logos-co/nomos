use crate::Result;

pub trait WitnessGenerator {
    /// Generates a witness from the given inputs.
    ///
    /// # Arguments
    ///
    /// * `inputs` - A string containing the public and private inputs.
    ///
    /// # Returns
    ///
    /// A [`Result<Vec<u8>>`] which contains the witness if successful.
    fn generate_witness(inputs: &str) -> Result<Vec<u8>>;
}
