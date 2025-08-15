use std::path::PathBuf;

const CARGO_MANIFEST_DIR: &str = "CARGO_MANIFEST_DIR";
const CRATE_BIN_PATH: &str = "bin";
const PATH_ENV_VAR: &str = "PATH";
const PATH_SEPARATOR: &str = ":";

/// Check if a binary exists in the crate's bin directory
///
/// # Arguments
///
/// * `binary_name` - The name of the binary to check.
#[must_use]
pub fn find_binary_in_crate(binary_name: &str) -> Option<PathBuf> {
    let path = std::env::var(CARGO_MANIFEST_DIR).ok().map(|directory| {
        PathBuf::from(directory)
            .join(CRATE_BIN_PATH)
            .join(binary_name)
    })?;

    path.is_file().then_some(path)
}

/// Check if a binary exists in an environment variable
///
/// # Arguments
///
/// * `binary_name` - The name of the binary to check.
/// * `environment_variable` - The name of the environment variable that may
///   contain the binary directory or path.
#[must_use]
pub fn find_binary_in_environment_variable(
    binary_name: &str,
    environment_variable: &str,
) -> Option<PathBuf> {
    let path = std::env::var(environment_variable)
        .ok()
        .map(PathBuf::from)?;

    if path.is_file() {
        return Some(path);
    }

    let binary_path = path.join(binary_name);
    binary_path.is_file().then_some(binary_path)
}

/// Check if a binary exists in the system PATH
///
/// # Arguments
///
/// * `binary_name` - The name of the binary to check.
#[must_use]
pub fn find_binary_in_path(binary_name: &str) -> Option<PathBuf> {
    std::env::var(PATH_ENV_VAR)
        .ok()?
        .split(PATH_SEPARATOR)
        .map(|dir| PathBuf::from(dir).join(binary_name))
        .find(|path| path.is_file())
}

/// Find the binary
/// This function checks the repository's bin directory, an environment
/// variable, and finally falls back to the system PATH.
///
/// # Arguments
///
/// * `binary_name` - The name of the binary to find.
/// * `environment_variable` - The name of the environment variable that may
///   contain the binary directory or path.
///
/// # Returns
///
/// An `Option<PathBuf>` that contains the path to the binary if found, or
/// `None` if not found.
pub fn find_binary(binary_name: &str, environment_variable: &str) -> Result<PathBuf, String> {
    let binary = find_binary_in_crate(binary_name)
        .or_else(|| find_binary_in_environment_variable(binary_name, environment_variable))
        .or_else(|| find_binary_in_path(binary_name));

    binary.ok_or_else(||
        format!(
            "Binary '{binary_name}' not found in the crate-relative 'bin/' directory, environment variable ({environment_variable}), or system PATH.",
        )
    )
}
