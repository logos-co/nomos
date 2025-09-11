use std::{path::PathBuf, sync::LazyLock};

const POC_PROVING_KEY_NAME: &str = "poc.zkey";
const NOMOS_POC_PROVING_KEY_ENVAR: &str = "NOMOS_POC_PROVING_KEY_PATH";

pub static POC_PROVING_KEY_PATH: LazyLock<PathBuf> = LazyLock::new(|| {
    circuits_utils::find_file(POC_PROVING_KEY_NAME, NOMOS_POC_PROVING_KEY_ENVAR)
        .unwrap_or_else(|_| panic!("{POC_PROVING_KEY_NAME} should not be missing"))
});
