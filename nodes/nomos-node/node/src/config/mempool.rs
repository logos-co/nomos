use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MempoolConfig {
    pub cl_pool_recovery_path: PathBuf,
    pub da_pool_recovery_path: PathBuf,
}
