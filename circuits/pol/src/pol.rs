use std::io;

use crate::{WitnessGenerator, pol_from_content};

pub struct Pol;

impl WitnessGenerator for Pol {
    fn generate_witness(inputs: &str) -> io::Result<Vec<u8>> {
        pol_from_content(inputs)
    }
}
