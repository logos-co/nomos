use std::{
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, Result};
use tempfile::TempDir;

/// Copy the repository `testnet/` directory into a scenario-specific temp dir.
#[derive(Debug)]
pub struct ComposeWorkspace {
    root: TempDir,
}

impl ComposeWorkspace {
    /// Clone the testnet assets into a temporary directory.
    pub fn create() -> Result<Self> {
        let repo_root = env::var("CARGO_WORKSPACE_DIR")
            .map(PathBuf::from)
            .or_else(|_| {
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .parent()
                    .and_then(Path::parent)
                    .and_then(Path::parent)
                    .map(Path::to_path_buf)
                    .context("resolving workspace root from manifest dir")
            })
            .context("locating repository root")?;
        let temp = tempfile::Builder::new()
            .prefix("nomos-testnet-")
            .tempdir()
            .context("creating testnet temp dir")?;
        let testnet_source = repo_root.join("testnet");
        if !testnet_source.exists() {
            anyhow::bail!(
                "testnet directory not found at {}",
                testnet_source.display()
            );
        }
        copy_dir_recursive(&testnet_source, &temp.path().join("testnet"))?;

        let kzg_source = repo_root.join("tests/kzgrs/kzgrs_test_params");
        if kzg_source.exists() {
            let target = temp.path().join("kzgrs_test_params");
            if kzg_source.is_dir() {
                copy_dir_recursive(&kzg_source, &target)?;
            } else {
                fs::copy(&kzg_source, &target).with_context(|| {
                    format!("copying {} -> {}", kzg_source.display(), target.display())
                })?;
            }
        }

        Ok(Self { root: temp })
    }

    #[must_use]
    pub fn root_path(&self) -> &Path {
        self.root.path()
    }

    #[must_use]
    pub fn testnet_dir(&self) -> PathBuf {
        self.root.path().join("testnet")
    }

    #[must_use]
    pub fn into_inner(self) -> TempDir {
        self.root
    }
}

fn copy_dir_recursive(source: &Path, target: &Path) -> Result<()> {
    fs::create_dir_all(target)
        .with_context(|| format!("creating target dir {}", target.display()))?;
    for entry in fs::read_dir(source).with_context(|| format!("reading {}", source.display()))? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let dest = target.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &dest)?;
        } else if !file_type.is_dir() {
            fs::copy(entry.path(), &dest).with_context(|| {
                format!("copying {} -> {}", entry.path().display(), dest.display())
            })?;
        }
    }
    Ok(())
}
