mod api_client;
pub mod executor;
pub mod validator;

use std::sync::LazyLock;

pub use api_client::ApiClient;
use tempfile::TempDir;

pub(crate) const LOGS_PREFIX: &str = "__logs";
static KEEP_NODE_TEMPDIRS: LazyLock<bool> =
    LazyLock::new(|| std::env::var("NOMOS_TESTS_KEEP_LOGS").is_ok());

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

pub(crate) fn should_persist_tempdir() -> bool {
    std::thread::panicking() || *KEEP_NODE_TEMPDIRS
}
