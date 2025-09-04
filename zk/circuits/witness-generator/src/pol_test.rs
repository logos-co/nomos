#[cfg(test)]
mod tests {
    use std::{fs::read_to_string, io::Write as _, path::PathBuf, sync::LazyLock};

    use circuits_utils::find_binary;
    use tempfile::NamedTempFile;

    use crate::{generate_witness, generate_witness_from_paths};

    const BINARY_NAME: &str = "pol";
    const BINARY_ENV_VAR: &str = "NOMOS_POL";

    static BINARY: LazyLock<PathBuf> = LazyLock::new(|| {
        find_binary(BINARY_NAME, BINARY_ENV_VAR).unwrap_or_else(|error_message| {
            panic!("Could not find the required '{BINARY_NAME}' binary: {error_message}");
        })
    });

    static INPUT: LazyLock<PathBuf> = LazyLock::new(|| {
        let file = PathBuf::from("resources/tests/input.json");
        assert!(file.exists(), "Could not find {}.", file.display());
        file
    });

    #[test]
    fn test_pol() {
        let witness_file = NamedTempFile::new().unwrap();
        let result = generate_witness_from_paths(
            &INPUT,
            &witness_file.path().to_path_buf(),
            BINARY.as_path(),
        )
        .unwrap();
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
            BINARY.as_path(),
        );
        assert!(result.is_err(), "Expected pol to fail with invalid input");
    }

    #[test]
    fn test_pol_from_content() {
        let input = read_to_string(&*INPUT).unwrap();
        let witness = generate_witness(input.as_ref(), BINARY.as_path()).unwrap();
        assert!(!witness.is_empty(), "The witness should not be empty");
    }

    #[test]
    fn test_pol_from_content_invalid() {
        let invalid_input = "invalid input";
        let result = generate_witness(invalid_input, BINARY.as_path());
        assert!(
            result.is_err(),
            "Expected pol_from_content to fail with invalid input"
        );
    }
}
