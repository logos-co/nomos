pub mod executor;
pub mod validator;

use std::{ops::Range, sync::LazyLock};

use reqwest::Client;
use serde::{Deserialize, Serialize};
use tempfile::TempDir;

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

#[derive(Serialize, Deserialize)]
struct GetRangeReq {
    pub app_id: [u8; 32],
    pub range: Range<[u8; 8]>,
}
