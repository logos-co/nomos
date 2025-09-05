use std::{path::PathBuf, sync::LazyLock};

const POQ_PROVING_KEY_NAME: &str = "poq.zkey";
const NOMOS_POQ_PROVING_KEY_ENVAR: &str = "NOMOS_POQ_PROVING_KEY_PATH";

pub static POQ_PROVING_KEY_PATH: LazyLock<PathBuf> = LazyLock::new(|| {
    circuits_utils::find_binary(POQ_PROVING_KEY_NAME, NOMOS_POQ_PROVING_KEY_ENVAR)
        .unwrap_or_else(|_| panic!("{POQ_PROVING_KEY_NAME} should not be missing"))
});
