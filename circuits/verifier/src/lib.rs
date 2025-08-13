use std::{io::Write as _, path::PathBuf, sync::LazyLock};

use circuits_utils::find_binary;
use tempfile::NamedTempFile;

const BINARY_NAME: &str = "verifier";
const BINARY_ENV_VAR: &str = "NOMOS_VERIFIER";

static BINARY: LazyLock<PathBuf, fn() -> PathBuf> = LazyLock::new(|| {
    find_binary(BINARY_NAME, BINARY_ENV_VAR)
        .unwrap_or_else(|error_message| panic!("{}", error_message))
});

/// Runs the `verifier` command to check the validity of a proof for a given
/// verification key and public inputs.
///
/// # Arguments
///
/// * `verification_key_file` - The path to the verification key file.
/// * `public_file` - The path to the public inputs file.
/// * `proof_file` - The path to the proof file.
///
/// # Returns
///
/// An `io::Result<bool>` which indicates whether the verification was
/// successful or not, or an `io::Error` if the command fails.
fn verifier(
    verification_key_file: &PathBuf,
    public_file: &PathBuf,
    proof_file: &PathBuf,
) -> std::io::Result<bool> {
    let output = std::process::Command::new(BINARY.to_owned())
        .arg(verification_key_file)
        .arg(public_file)
        .arg(proof_file)
        .output()?;

    Ok(output.status.success())
}

/// Runs the `verifier` command to check the validity of a proof for a given
/// verification key and public inputs.
///
/// # Note
///
/// Calls [`verifier`] underneath but hides the file handling details.
///
/// # Arguments
///
/// * `verification_key_contents` - The contents of the verification key.
/// * `public_contents` - The contents of the public inputs.
/// * `proof_contents` - The contents of the proof.
///
/// # Returns
///
/// An `io::Result<bool>` which indicates whether the verification was
/// successful or not, or an `io::Error` if the command fails.
pub fn verifier_from_contents(
    verification_key_contents: &str,
    public_contents: &str,
    proof_contents: &str,
) -> std::io::Result<bool> {
    let mut verification_key_file = NamedTempFile::new()?;
    let mut public_file = NamedTempFile::new()?;
    let mut proof_file = NamedTempFile::new()?;
    verification_key_file.write_all(verification_key_contents.as_bytes())?;
    public_file.write_all(public_contents.as_bytes())?;
    proof_file.write_all(proof_contents.as_bytes())?;

    verifier(
        &verification_key_file.path().to_path_buf(),
        &public_file.path().to_path_buf(),
        &proof_file.path().to_path_buf(),
    )
}
