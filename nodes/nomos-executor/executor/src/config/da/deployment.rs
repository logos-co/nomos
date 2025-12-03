use core::time::Duration;

pub use nomos_node::config::da::deployment::{
    CommonSettings, DispersalSettings, NetworkSettings, SamplingSettings, VerifierSettings,
};
use serde::{Deserialize, Serialize};

use crate::config::deployment::Settings as DeploymentSettings;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Settings {
    #[serde(flatten)]
    pub validator: nomos_node::config::da::deployment::Settings,
    pub dispersal: DispersalSettings,
}

impl From<DeploymentSettings> for Settings {
    fn from(value: DeploymentSettings) -> Self {
        match value {
            DeploymentSettings::Mainnet => mainnet_settings(),
            DeploymentSettings::Custom(custom) => custom.da,
        }
    }
}

fn mainnet_settings() -> Settings {
    let validator = nomos_node::config::da::deployment::mainnet_settings();

    // Add executor-specific dispersal settings
    let dispersal = DispersalSettings {
        dispersal_timeout: Duration::from_secs(20),
        retry_cooldown: Duration::from_secs(5),
        retry_limit: 2,
    };

    Settings {
        validator,
        dispersal,
    }
}
