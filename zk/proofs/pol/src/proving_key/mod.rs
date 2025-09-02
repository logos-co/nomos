use std::{path::Path, sync::LazyLock};

pub static POL_PROVING_KEY_PATH: LazyLock<&Path> = LazyLock::new(|| {
    Path::new("/Users/netwave/projects/rust/nomos-node/zk/proofs/pol/src/proving_key/pol.zkey")
});
