mod mixnode;
pub mod nomos;

pub use self::mixnode::MixNode;
pub use nomos::NomosNode;
use tempfile::TempDir;

const LOGS_PREFIX: &str = "__logs";

fn create_tempdir() -> std::io::Result<TempDir> {
    // It's easier to use the current location instead of OS-default tempfile location
    // because Github Actions can easily access files in the current location using wildcard
    // to upload them as artifacts.
    tempfile::TempDir::new_in(std::env::current_dir()?)
}

fn persist_tempdir(tempdir: &mut TempDir, label: &str) -> std::io::Result<()> {
    println!(
        "{}: persisting directory at {}",
        label,
        tempdir.path().display()
    );
    // we need ownership of the dir to persist it
    let dir = std::mem::replace(tempdir, tempfile::tempdir()?);
    // a bit confusing but `into_path` persists the directory
    let _ = dir.into_path();
    Ok(())
}
