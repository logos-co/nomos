use crate::{WitnessGenerator, wrappers::pol_from_content};

pub struct Pol;

impl WitnessGenerator for Pol {
    type Error = std::io::Error;

    fn generate_witness(inputs: &str) -> Result<Vec<u8>, Self::Error> {
        pol_from_content(inputs)
    }
}
