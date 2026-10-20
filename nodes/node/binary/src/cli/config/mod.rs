pub mod init;
pub mod keystore;
pub mod merge;
pub mod migrate;
pub mod migrate_0_1_2;
pub mod update;

use std::{fs, io::Write as _, path::Path};

// Secret files are created owner only and never through a symlink.
fn secret_file_options() -> fs::OpenOptions {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;

        options.mode(0o600).custom_flags(libc::O_NOFOLLOW)
    };
    options
}

pub fn write_new_secret_file(path: &Path, contents: &str) -> Result<(), std::io::Error> {
    secret_file_options()
        .open(path)?
        .write_all(contents.as_bytes())
}

pub fn replace_secret_file(path: &Path, contents: &str) -> Result<(), std::io::Error> {
    let mut temp_name = path.file_name().unwrap_or_default().to_os_string();
    temp_name.push(format!(".{}.tmp", std::process::id()));
    let temp_path = path.with_file_name(temp_name);
    let result =
        write_new_secret_file(&temp_path, contents).and_then(|()| fs::rename(&temp_path, path));
    if result.is_err() {
        drop(fs::remove_file(&temp_path));
    }
    result
}

pub fn confirm_overwrite(msg: &str) -> Result<bool, std::io::Error> {
    print!("{msg} (y/N): ");
    std::io::stdout().flush()?;

    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;

    let trimmed = input.trim().to_lowercase();
    Ok(trimmed == "y" || trimmed == "yes")
}
