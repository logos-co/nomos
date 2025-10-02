pub mod executor;
pub mod validator;

use std::sync::LazyLock;

use reqwest::Client;
use tempfile::TempDir;

const DA_GET_TESTING_ENDPOINT_ERROR: &str = "Failed to connect to testing endpoint. The binary was likely built without the 'testing' feature. Try: cargo build --workspace --all-features";

const LOGS_PREFIX: &str = "__logs";
static CLIENT: LazyLock<Client> = LazyLock::new(Client::new);

fn create_tempdir() -> std::io::Result<TempDir> {
    // It's easier to use the current location instead of OS-default tempfile
    // location because Github Actions can easily access files in the current
    // location using wildcard to upload them as artifacts.
    TempDir::new_in(std::env::current_dir()?)
}

fn persist_tempdir(tempdir: &mut TempDir, label: &str) -> std::io::Result<()> {
    println!(
        "{}: persisting directory at {}",
        label,
        tempdir.path().display()
    );
    // we need ownership of the dir to persist it
    let dir = std::mem::replace(tempdir, tempfile::tempdir()?);
    let _ = dir.keep();
    Ok(())
}
