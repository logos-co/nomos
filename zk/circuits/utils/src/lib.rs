use std::path::{Path, PathBuf};

const CARGO_MANIFEST_DIR: &str = "CARGO_MANIFEST_DIR";
const CRATE_RELATIVE_BIN_PATH: &str = "bin";
const PATH_ENV_VAR: &str = "PATH";

/// Find a file in a specified directory.
///
/// # Arguments
///
/// * `directory` - The directory to search in.
/// * `file_name` - The name of the file to find.
///
/// # Returns
///
/// An `Option<PathBuf>` that contains the path to the file if found.
#[must_use]
pub fn find_file_in_directory(directory: &Path, file_name: &str) -> Option<PathBuf> {
    let path = directory.join(file_name);
    path.is_file().then_some(path)
}

/// Find a file in an environment variable.
///
/// If the environment variable points to a file directly, it will be used,
/// regardless of the file name. If it points to a directory, the function will
/// look for the file within that directory using the provided file name.
///
/// # Arguments
///
/// * `file_name` - The name of the file to find (only used if the environment
///   variable points to a directory).
/// * `environment_variable` - The name of the environment variable that may
///   point to a file or a directory containing the file.
///
/// # Returns
///
/// An `Option<PathBuf>` that contains the path to the file if found.
#[must_use]
pub fn find_file_in_environment_variable(
    file_name: &str,
    environment_variable: &str,
) -> Option<PathBuf> {
    let path = std::env::var_os(environment_variable).map(PathBuf::from)?;

    if path.is_file() {
        return Some(path);
    }

    let file_path = path.join(file_name);
    file_path.is_file().then_some(file_path)
}

/// Find a file in the system PATH.
///
/// # Arguments
///
/// * `file_name` - The name of the file to find.
///
/// # Returns
/// An `Option<PathBuf>` that contains the path to the file if found.
#[must_use]
pub fn find_file_in_path(file_name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os(PATH_ENV_VAR)?;
    std::env::split_paths(&paths)
        .map(|dir| dir.join(file_name))
        .find(|path| path.is_file())
}

/// Get the crate-relative `bin/` directory.
///
/// # Returns
///
/// A `Result<PathBuf, String>` that contains the path to the crate's `bin/`
/// directory if the `CARGO_MANIFEST_DIR` environment variable is set, or an
/// error message if it is not.
fn get_crate_bin() -> Result<PathBuf, String> {
    std::env::var(CARGO_MANIFEST_DIR)
        .map(|dir| PathBuf::from(dir).join(CRATE_RELATIVE_BIN_PATH))
        .map_err(|error| format!("Environment variable {CARGO_MANIFEST_DIR} is not set: {error}"))
}

/// Find a file by checking multiple locations.
///
/// This function checks, in order, the specified environment variable, the
/// system PATH, and the local bin directory to the crate where this was called.
///
/// If the environment variable points to a file directly, it will be used,
/// regardless of the file name.
///
/// # Arguments
///
/// * `file_name` - The name of the file to find.
/// * `environment_variable` - The name of the environment variable that may
///   contain the file directory or path.
///
/// # Returns
///
/// An `Option<PathBuf>` that contains the path to the file if found.
pub fn find_file(file_name: &str, environment_variable: &str) -> Result<PathBuf, String> {
    if let Some(path) = find_file_in_environment_variable(file_name, environment_variable) {
        return Ok(path);
    }

    if let Some(path) = find_file_in_path(file_name) {
        return Ok(path);
    }

    let crate_bin = get_crate_bin();
    if let Ok(bin_dir) = &crate_bin
        && let Some(path) = find_file_in_directory(bin_dir, file_name)
    {
        return Ok(path);
    }

    let display_crate_bin = crate_bin.map_or_else(|error| error, |path| path.display().to_string());
    Err(format!(
        "File '{file_name}' could not be found in the environment variable ({environment_variable}), system PATH, or crate-relative 'bin/' directory ({display_crate_bin})."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_file_in_directory() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_name = "file";
        let file_path = temp_dir.path().join(file_name);
        std::fs::write(&file_path, "content").unwrap();

        assert_eq!(
            find_file_in_directory(temp_dir.path(), file_name),
            Some(file_path)
        );
        assert_eq!(
            find_file_in_directory(temp_dir.path(), "non_existent_file"),
            None
        );
    }

    #[test]
    fn test_get_crate_bin() {
        let temp_dir = tempfile::tempdir().unwrap();
        let temp_dir_path = temp_dir.path().to_str();
        temp_env::with_vars([(CARGO_MANIFEST_DIR, temp_dir_path)], || {
            let expected_bin_path = temp_dir.path().join(CRATE_RELATIVE_BIN_PATH);
            assert_eq!(get_crate_bin().unwrap(), expected_bin_path);
        });
        temp_env::with_vars([(CARGO_MANIFEST_DIR, Option::<&str>::None)], || {
            let error = get_crate_bin().unwrap_err();
            assert!(error.contains("is not set"));
        });
    }

    #[test]
    fn test_find_file_in_environment_variable() {
        let env_var_name = "TEST_ENV_VAR";
        let file_name = "file";
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join(file_name);
        std::fs::write(&file_path, "content").unwrap();

        // Env var points to a directory: should look for the file within that directory
        temp_env::with_vars([(env_var_name, temp_dir.path().to_str())], || {
            assert_eq!(
                find_file_in_environment_variable(file_name, env_var_name),
                Some(file_path.clone())
            );
            assert_eq!(
                find_file_in_environment_variable("non_existent_file", env_var_name),
                None
            );
        });

        // Env var points to an existing file: should return that file regardless of the
        // file name provided
        temp_env::with_vars([(env_var_name, file_path.clone().to_str())], || {
            assert_eq!(
                find_file_in_environment_variable(file_name, env_var_name),
                Some(file_path.clone())
            );
            assert_eq!(
                find_file_in_environment_variable("non_existent_file", env_var_name),
                Some(file_path)
            );
        });

        // Env var points to a non-existent path: should return None
        let non_existent_path = temp_dir.path().join("non_existent");
        temp_env::with_vars([(env_var_name, Some(non_existent_path))], || {
            assert_eq!(
                find_file_in_environment_variable(file_name, env_var_name),
                None
            );
        });
    }

    #[test]
    fn test_find_file_in_path() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("file_with_unusual_name");
        std::fs::write(&file_path, "content").unwrap();

        let original_path = std::env::var_os(PATH_ENV_VAR).unwrap_or_default();
        let new_path = std::env::join_paths(
            std::env::split_paths(&original_path).chain([temp_dir.path().to_path_buf()]),
        )
        .expect("Could not join paths");

        temp_env::with_vars([(PATH_ENV_VAR, new_path.to_str())], || {
            assert_eq!(find_file_in_path("file_with_unusual_name"), Some(file_path));
            assert_eq!(find_file_in_path("non_existent_file"), None);
        });
    }

    /// Comprehensive test for [`find_file`] function, checking all search
    /// locations in order.
    ///
    /// This test configures the environment incrementally to ensure that the
    /// function correctly prioritises the search locations, from highest to
    /// lowest priority:
    /// 1. Environment variable pointing to the file directly.
    /// 2. System PATH.
    /// 3. Crate-relative `bin/` directory.
    /// 4. Failure to find the file in any location.
    ///
    /// The test doesn't check what internal function
    /// was used to find the file, but implicitly trusts that if the file is
    /// found, it was found in the correct location due to the order of
    /// environment setup.
    #[test]
    fn test_find_file() {
        let env_var_name = "TEST_ENV_VAR_FOR_FIND_FILE";
        let file_name = "file_with_unusual_name";
        let temp_dir = tempfile::tempdir().unwrap();
        let temp_dir_path = temp_dir.path();
        let file_path = temp_dir.path().join(file_name);
        std::fs::write(&file_path, "content").unwrap();

        // Set up the crate's bin directory environment (while keeping env var and PATH
        // unset)
        temp_env::with_vars([(CARGO_MANIFEST_DIR, temp_dir_path.to_str())], || {
            let bin_dir = temp_dir_path.join(CRATE_RELATIVE_BIN_PATH);
            std::fs::create_dir_all(&bin_dir).unwrap();
            let bin_file_path = bin_dir.join(file_name);
            std::fs::write(&bin_file_path, "content").unwrap();
            assert_eq!(find_file(file_name, env_var_name).unwrap(), bin_file_path);

            // Set up the system PATH environment (while keeping the crate's bin directory
            // in PATH)
            let original_path = std::env::var_os(PATH_ENV_VAR).unwrap_or_default();
            let new_path = std::env::join_paths(
                std::env::split_paths(&original_path).chain([temp_dir_path.to_path_buf()]),
            )
            .expect("Could not join paths");
            let path_file_path = temp_dir_path.join(file_name);
            temp_env::with_vars([(PATH_ENV_VAR, new_path.to_str())], || {
                assert_eq!(
                    find_file(file_name, env_var_name).unwrap(),
                    path_file_path.clone()
                );

                // Set up the environment variable environment (while keeping PATH and crate's
                // bin directory intact)
                temp_env::with_vars([(env_var_name, file_path.to_str())], || {
                    assert_eq!(
                        find_file(file_name, env_var_name).unwrap(),
                        file_path.clone()
                    );
                });
            });
        });

        // File could not be found anywhere
        let file = find_file("non_existent_file", env_var_name);
        assert!(file.is_err());
        assert!(file.unwrap_err().contains("could not be found"));
    }
}
