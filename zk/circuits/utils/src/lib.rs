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

/// Find a file in the crate's bin directory
///
/// # Arguments
///
/// * `file_name` - The name of the file to find.
///
/// # Returns
///
/// An `Option<PathBuf>` that contains the path to the file if found.
#[must_use]
pub fn find_file_in_crate_bin(file_name: &str) -> Option<PathBuf> {
    let crate_bin_path = std::env::var(CARGO_MANIFEST_DIR)
        .ok()
        .map(|dir| PathBuf::from(dir).join(CRATE_RELATIVE_BIN_PATH))?;
    find_file_in_directory(&crate_bin_path, file_name)
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

/// Find a file by checking multiple locations.
///
/// This function checks the repository's bin directory, an environment
/// variable, and finally falls back to the system PATH.
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
    let file = find_file_in_environment_variable(environment_variable, file_name)
        .or_else(|| find_file_in_path(file_name))
        .or_else(|| find_file_in_crate_bin(file_name));

    file.ok_or_else(||
        format!(
            "File '{file_name}' could not be found in the crate-relative 'bin/' directory, environment variable ({environment_variable}), or system PATH.",
        )
    )
}
