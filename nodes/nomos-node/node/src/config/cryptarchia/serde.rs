use std::path::PathBuf;

use chain_service::{BootstrapConfig, StartingState};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub starting_state: StartingState,
    pub recovery_file: PathBuf,
    pub bootstrap: BootstrapConfig,
}
