use std::{path::Path, sync::LazyLock};

static POL_PROVING_KEY_PATH: LazyLock<&Path> = LazyLock::new(|| Path::new("./pol.zkey"));
