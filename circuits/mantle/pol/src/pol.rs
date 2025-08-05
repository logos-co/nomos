use crate::{Result, WitnessGenerator, wrappers::pol_from_content};

pub struct Pol;

impl WitnessGenerator for Pol {
    fn generate_witness(inputs: &str) -> Result<Vec<u8>> {
        pol_from_content(inputs)
    }
}
