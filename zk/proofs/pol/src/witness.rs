use std::error::Error;

use pol_witness_generator::PolWitnessGenerator;
use witness_generator_core::WitnessGenerator as _;

use crate::{PolInputs, inputs::PolInputsJson};

pub struct Witness(Vec<u8>);

impl Witness {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn into_inner(self) -> Vec<u8> {
        self.0
    }
}

impl AsRef<[u8]> for Witness {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

fn generate_witness(inputs: PolInputs) -> Result<Witness, impl Error> {
    let pol_inputs_json: PolInputsJson = inputs.into();
    let str_inputs: String =
        serde_json::to_string(&pol_inputs_json).expect("Failed to serialize inputs");
    PolWitnessGenerator::generate_witness(&str_inputs).map(Witness)
}
