include!("../utils/src/lib.rs");

const BINARY_NAME: &str = "prover";
const BINARY_ENV_VAR: &str = "NOMOS_BIN_PROVER";

fn main() {
    if find_binary(BINARY_NAME, BINARY_ENV_VAR).is_none() {
        eprintln!("The binary '{BINARY_NAME}' could not be found.");
        std::process::exit(1);
    }
}
