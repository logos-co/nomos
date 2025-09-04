use std::{
    io::{Error, Result, Write as _},
    path::PathBuf,
    sync::LazyLock,
};

use circuits_utils::find_binary;
use tempfile::NamedTempFile;

const BINARY_NAME: &str = "pol";
const BINARY_ENV_VAR: &str = "NOMOS_POL";

static BINARY: LazyLock<PathBuf> = LazyLock::new(|| {
    find_binary(BINARY_NAME, BINARY_ENV_VAR).unwrap_or_else(|error_message| {
        panic!("Could not find the required '{BINARY_NAME}' binary: {error_message}");
    })
});

/// Runs the `pol` circuit to generate a witness from the provided inputs.
///
/// # Arguments
///
/// * `inputs_file` - The path to the file containing the public and private
///   inputs.
/// * `witness_file` - The path to the file where the witness will be written.
///
/// # Returns
///
/// A [`Result`] which contains the path to the witness file if successful.
fn generate_witness_from_paths(inputs_file: &PathBuf, witness_file: &PathBuf) -> Result<PathBuf> {
    let output = std::process::Command::new(BINARY.to_owned())
        .arg(inputs_file)
        .arg(witness_file)
        .output()?;

    if !output.status.success() {
        let error_message = String::from_utf8_lossy(&output.stderr);
        return Err(Error::other(format!("pol command failed: {error_message}")));
    }

    Ok(witness_file.to_owned())
}

/// Runs the `pol` circuit to generate a witness from the provided inputs.
///
/// # Note
///
/// Calls [`crate::pol`] underneath but hides the file handling details.
///
/// # Arguments
///
/// * `inputs` - A string containing the public and private inputs.
///
/// # Returns
///
/// A [`Result`] which contains the witness if successful.
pub fn generate_witness(inputs: &str) -> Result<Vec<u8>> {
    let mut inputs_file = NamedTempFile::new()?;
    let witness_file = NamedTempFile::new()?;
    inputs_file.write_all(inputs.as_bytes())?;

    generate_witness_from_paths(
        &inputs_file.path().to_path_buf(),
        &witness_file.path().to_path_buf(),
    )?;
    std::fs::read(witness_file.path())
}

#[cfg(test)]
mod tests {
    use std::fs::read_to_string;

    use super::*;

    static INPUT: LazyLock<PathBuf> = LazyLock::new(|| {
        let file = PathBuf::from("resources/tests/input.json");
        assert!(file.exists(), "Could not find {}.", file.display());
        file
    });

    #[test]
    fn test_pol() {
        let witness_file = NamedTempFile::new().unwrap();
        let result =
            generate_witness_from_paths(&INPUT, &witness_file.path().to_path_buf()).unwrap();
        assert_eq!(
            result,
            witness_file.path(),
            "The witness file path should match the expected path"
        );

        let witness_content = std::fs::read(witness_file.path()).unwrap();
        assert!(
            !witness_content.is_empty(),
            "The witness file should not be empty"
        );
    }

    #[test]
    fn test_pol_invalid_input() {
        let mut invalid_input_file = NamedTempFile::new().unwrap();
        invalid_input_file.write_all(b"invalid input").unwrap();
        let witness_file = NamedTempFile::new().unwrap();
        let result = generate_witness_from_paths(
            &invalid_input_file.path().to_path_buf(),
            &witness_file.path().to_path_buf(),
        );
        assert!(result.is_err(), "Expected pol to fail with invalid input");
    }

    #[test]
    fn test_pol_from_content() {
        let input = read_to_string(&*INPUT).unwrap();
        let witness = generate_witness(input.as_ref()).unwrap();
        assert!(!witness.is_empty(), "The witness should not be empty");
    }

    #[test]
    fn test_pol_from_content_invalid() {
        let invalid_input = "invalid input";
        let result = generate_witness(invalid_input);
        assert!(
            result.is_err(),
            "Expected pol_from_content to fail with invalid input"
        );
    }
}
