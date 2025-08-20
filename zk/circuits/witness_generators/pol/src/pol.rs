use witness_generator_core::WitnessGenerator;

use crate::wrappers::pol_from_content;
pub struct PolWitnessGenerator;

impl WitnessGenerator for PolWitnessGenerator {
    type Error = std::io::Error;

    fn generate_witness(inputs: &str) -> Result<Vec<u8>, Self::Error> {
        pol_from_content(inputs)
    }
}
