use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MempoolConfig {
    pub pool_recovery_path: PathBuf,
}
