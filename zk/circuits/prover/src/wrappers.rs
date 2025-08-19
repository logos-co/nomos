use std::{
    io::{Error, Result, Write as _},
    path::PathBuf,
    sync::LazyLock,
};

use circuits_utils::find_binary;
use tempfile::NamedTempFile;

const BINARY_NAME: &str = "prover";
const BINARY_ENV_VAR: &str = "NOMOS_PROVER";

static BINARY: LazyLock<PathBuf> = LazyLock::new(|| {
    find_binary(BINARY_NAME, BINARY_ENV_VAR).unwrap_or_else(|error_message| {
        panic!("Could not find the required '{BINARY_NAME}' binary: {error_message}");
    })
});

/// Runs the `prover` command to generate a proof and public inputs for the
/// given circuit and witness contents.
///
/// # Arguments
///
/// * `circuit_file` - The path to the file containing the circuit (proving
///   key).
/// * `witness_file` - The path to the file containing the witness.
/// * `proof_file` - The path to the file where the proof will be written.
/// * `public_file` - The path to the file where the public inputs will be
///   written.
///
/// # Returns
///
/// A [`Result`] which contains the paths to the proof file and public inputs
/// file if successful.
pub fn prover(
    circuit_file: &PathBuf,
    witness_file: &PathBuf,
    proof_file: &PathBuf,
    public_file: &PathBuf,
) -> Result<(PathBuf, PathBuf)> {
    let output = std::process::Command::new(BINARY.to_owned())
        .arg(circuit_file)
        .arg(witness_file)
        .arg(proof_file)
        .arg(public_file)
        .output()?;

    if !output.status.success() {
        let error_message = String::from_utf8_lossy(&output.stderr);
        return Err(Error::other(format!(
            "prover command failed: {error_message}"
        )));
    }

    Ok((proof_file.to_owned(), public_file.to_owned()))
}

/// Runs the `prover` command to generate a proof and public inputs for the
/// given circuit and witness contents.
///
/// # Note
///
/// Calls [`prover`] underneath but hides the file handling details.
///
/// # Arguments
///
/// * `circuit_contents` - A byte slice containing the circuit (proving key).
/// * `witness_contents` - A byte slice containing the witness.
///
/// # Returns
///
/// A [`Result`] which contains the proof and public inputs as strings if
/// successful.
pub fn prover_from_contents(
    circuit_contents: &[u8],
    witness_contents: &[u8],
) -> Result<(Vec<u8>, Vec<u8>)> {
    let mut circuit_file = NamedTempFile::new()?;
    let mut witness_file = NamedTempFile::new()?;
    let proof_file = NamedTempFile::new()?;
    let public_file = NamedTempFile::new()?;
    circuit_file.write_all(circuit_contents)?;
    witness_file.write_all(witness_contents)?;

    prover(
        &circuit_file.path().to_path_buf(),
        &witness_file.path().to_path_buf(),
        &proof_file.path().to_path_buf(),
        &public_file.path().to_path_buf(),
    )?;

    let proof = std::fs::read(proof_file)?;
    let public = std::fs::read(public_file)?;
    Ok((proof, public))
}
