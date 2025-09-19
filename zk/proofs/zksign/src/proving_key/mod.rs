use std::{path::PathBuf, sync::LazyLock};

const ZKSIGN_PROVING_KEY_NAME: &str = "zksign.zkey";
const NOMOS_ZKSIGN_PROVING_KEY_ENVAR: &str = "NOMOS_ZKSIGN_PROVING_KEY_PATH";

pub static ZKSIGN_PROVING_KEY_PATH: LazyLock<PathBuf> = LazyLock::new(|| {
    circuits_utils::find_file(ZKSIGN_PROVING_KEY_NAME, NOMOS_ZKSIGN_PROVING_KEY_ENVAR)
        .unwrap_or_else(|_| panic!("{ZKSIGN_PROVING_KEY_NAME} should not be missing"))
});
