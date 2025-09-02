use std::{path::PathBuf, sync::LazyLock};

const POL_PROVING_KEY_NAME: &str = "pol.zkey";
const NOMOS_POL_PROVING_KEY_ENVAR: &str = "NOMOS_POL_PROVING_KEY_PATH";

pub static POL_PROVING_KEY_PATH: LazyLock<PathBuf> = LazyLock::new(|| {
    circuits_utils::find_binary(POL_PROVING_KEY_NAME, NOMOS_POL_PROVING_KEY_ENVAR)
        .unwrap_or_else(|_| panic!("{POL_PROVING_KEY_NAME} should not be missing"))
});
