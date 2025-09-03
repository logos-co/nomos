use std::path::PathBuf;

const CARGO_MANIFEST_DIR: &str = "CARGO_MANIFEST_DIR";
const CRATE_BIN_PATH: &str = "bin";
const PATH_ENV_VAR: &str = "PATH";
const PATH_SEPARATOR: &str = ":";

/// Find a file in the crate's bin directory
///
/// # Arguments
///
/// * `file_name` - The name of the file to find.
#[must_use]
pub fn find_file_in_crate(file_name: &str) -> Option<PathBuf> {
    let path = std::env::var(CARGO_MANIFEST_DIR).ok().map(|directory| {
        PathBuf::from(directory)
            .join(CRATE_BIN_PATH)
            .join(file_name)
    })?;

    path.is_file().then_some(path)
}

/// Find a file in an environment variable
///
/// # Arguments
///
/// * `file_name` - The name of the file to check.
/// * `environment_variable` - The name of the environment variable that may
///   contain the file directory or path.
#[must_use]
pub fn find_file_in_environment_variable(
    file_name: &str,
    environment_variable: &str,
) -> Option<PathBuf> {
    let path = std::env::var(environment_variable)
        .ok()
        .map(PathBuf::from)?;

    if path.is_file() {
        return Some(path);
    }

    let file_path = path.join(file_name);
    file_path.is_file().then_some(file_path)
}

/// Find a file in the system PATH
///
/// # Arguments
///
/// * `file_name` - The name of the file to check.
#[must_use]
pub fn find_file_in_path(file_name: &str) -> Option<PathBuf> {
    std::env::var(PATH_ENV_VAR)
        .ok()?
        .split(PATH_SEPARATOR)
        .map(|dir| PathBuf::from(dir).join(file_name))
        .find(|path| path.is_file())
}

/// Find a file by checking multiple locations.
///
/// This function checks the repository's bin directory, an environment
/// variable, and finally falls back to the system PATH.
///
/// # Arguments
///
/// * `file_name` - The name of the file to find.
/// * `environment_variable` - The name of the environment variable that may
///   contain the file directory or path.
///
/// # Returns
///
/// An `Option<PathBuf>` that contains the path to the file if found, or
/// `None` if not found.
pub fn find_file(file_name: &str, environment_variable: &str) -> Result<PathBuf, String> {
    let file = find_file_in_crate(file_name)
        .or_else(|| find_file_in_environment_variable(file_name, environment_variable))
        .or_else(|| find_file_in_path(file_name));

    file.ok_or_else(||
        format!(
            "File '{file_name}' could not be found in the crate-relative 'bin/' directory, environment variable ({environment_variable}), or system PATH.",
        )
    )
}
