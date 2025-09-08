use std::{path::PathBuf, sync::LazyLock};

use circuits_utils::find_binary;

use crate::inputs::{ZkSignWitnessInputs, ZkSignWitnessInputsJson};

const BINARY_NAME: &str = "zksign";
const BINARY_ENV_VAR: &str = "NOMOS_ZKSIGN";

static BINARY: LazyLock<PathBuf> = LazyLock::new(|| {
    find_binary(BINARY_NAME, BINARY_ENV_VAR).unwrap_or_else(|error_message| {
        panic!("Could not find the required '{BINARY_NAME}' binary: {error_message}");
    })
});

/// Witness of the circuit.
pub struct Witness(Vec<u8>);

impl Witness {
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl AsRef<[u8]> for Witness {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

pub fn generate_witness(inputs: &ZkSignWitnessInputs) -> Result<Witness, std::io::Error> {
    let inputs_json: ZkSignWitnessInputsJson = inputs.into();
    let str_inputs: String =
        serde_json::to_string(&inputs_json).expect("Failed to serialize inputs");
    witness_generator::generate_witness(&str_inputs, BINARY.as_path()).map(Witness)
}
