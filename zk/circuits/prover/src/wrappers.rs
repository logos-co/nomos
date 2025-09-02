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

#[cfg(test)]
mod tests {
    use super::*;

    static CIRCUIT_ZKEY: LazyLock<PathBuf> = LazyLock::new(|| {
        let file = PathBuf::from("../witness_generators/pol/resources/tests/pol.zkey");
        assert!(file.exists(), "Could not find {}.", file.display());
        file
    });

    static WITNESS_WTNS: LazyLock<PathBuf> = LazyLock::new(|| {
        let file = PathBuf::from("../witness_generators/pol/resources/tests/witness.wtns");
        assert!(file.exists(), "Could not find {}.", file.display());
        file
    });

    #[test]
    fn test_prover() {
        let circuit_file = CIRCUIT_ZKEY.clone();
        let witness_file = WITNESS_WTNS.clone();
        let proof_file = NamedTempFile::new().unwrap();
        let public_file = NamedTempFile::new().unwrap();

        let result = prover(
            &circuit_file,
            &witness_file,
            &proof_file.path().to_path_buf(),
            &public_file.path().to_path_buf(),
        )
        .unwrap();
        assert_eq!(
            result.0,
            proof_file.path().to_path_buf(),
            "The proof file path should match the expected path"
        );
        assert_eq!(
            result.1,
            public_file.path().to_path_buf(),
            "The public file path should match the expected path"
        );

        let proof_content = std::fs::read_to_string(proof_file.path()).unwrap();
        assert!(
            !proof_content.is_empty(),
            "The proof file should not be empty"
        );

        let public_content = std::fs::read_to_string(public_file.path()).unwrap();
        assert!(
            !public_content.is_empty(),
            "The public file should not be empty"
        );
    }

    #[test]
    fn test_prover_invalid_input() {
        let circuit_file = CIRCUIT_ZKEY.clone();
        let mut witness_file = NamedTempFile::new().unwrap();
        witness_file.write_all(b"invalid witness").unwrap();
        let proof_file = NamedTempFile::new().unwrap();
        let public_file = NamedTempFile::new().unwrap();

        let result = prover(
            &circuit_file,
            &witness_file.path().to_path_buf(),
            &proof_file.path().to_path_buf(),
            &public_file.path().to_path_buf(),
        );
        assert!(
            result.is_err(),
            "Expected prover to fail with invalid input"
        );
    }

    #[test]
    fn test_prover_from_contents() {
        let circuit_contents = std::fs::read(&*CIRCUIT_ZKEY).unwrap();
        let witness_contents = std::fs::read(&*WITNESS_WTNS).unwrap();

        let (proof, public) = prover_from_contents(&circuit_contents, &witness_contents).unwrap();
        assert!(!proof.is_empty(), "The proof should not be empty");
        assert!(!public.is_empty(), "The public inputs should not be empty");
    }

    #[test]
    fn test_prover_from_contents_invalid() {
        let invalid_circuit_contents = b"invalid circuit";
        let invalid_witness_contents = b"invalid witness";

        let result = prover_from_contents(invalid_circuit_contents, invalid_witness_contents);
        assert!(
            result.is_err(),
            "Expected prover_from_contents to fail with invalid input"
        );
    }
}
