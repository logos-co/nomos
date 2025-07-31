use std::{fs::read_to_string, io, io::Write as _, path::PathBuf, sync::LazyLock};

use circuits_utils::find_binary;
use tempfile::NamedTempFile;

const BINARY_NAME: &str = "pol";
const BINARY_ENV_VAR: &str = "NOMOS_POL";

static BINARY: LazyLock<PathBuf, fn() -> PathBuf> = LazyLock::new(|| {
    find_binary(BINARY_NAME, BINARY_ENV_VAR)
        .unwrap_or_else(|error_message| panic!("{}", error_message))
});

/// Runs the `pol` command.
///
/// # Arguments
///
/// * `inputs_file` - The path to the file containing the public and private
///   inputs.
/// * `witness_file` - The path to the file where the witness will be written.
///
/// # Returns
///
/// An `io::Result<PathBuf>` which contains the path to the witness file if
/// successful, or an `io::Error` if the command fails.
pub fn pol(inputs_file: &PathBuf, witness_file: &PathBuf) -> io::Result<PathBuf> {
    let output = std::process::Command::new(BINARY.to_owned())
        .arg(inputs_file)
        .arg(witness_file)
        .output()?;

    if !output.status.success() {
        let error_message = String::from_utf8_lossy(&output.stderr);
        return Err(io::Error::other(format!(
            "pol command failed: {error_message}"
        )));
    }

    Ok(witness_file.to_owned())
}

/// Shorthand for running the `pol` command with the contents, instead of files.
///
/// # Arguments
///
/// * `input` - A string containing the public and private inputs.
///
/// # Returns
///
/// An `io::Result<String>` which contains the witness if successful, or an
/// `io::Error` if the command fails.
pub fn pol_from_content(input: &str) -> io::Result<String> {
    let mut inputs_file = NamedTempFile::new()?;
    let witness_file = NamedTempFile::new()?;
    inputs_file.write_all(input.as_bytes())?;

    pol(
        &inputs_file.path().to_path_buf(),
        &witness_file.path().to_path_buf(),
    )?;
    read_to_string(witness_file.path())
}
