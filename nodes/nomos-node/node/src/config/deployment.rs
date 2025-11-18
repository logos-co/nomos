use serde::{Deserialize, Serialize};

use crate::config::blend::deployment::Settings as BlendDeploymentSettings;

/// Well-known deployments supported by the Nomos binary.
///
/// Any deployment different than any of the well-known falls under the `Custom`
/// category.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Settings {
    #[serde(rename = "mainnet")]
    Mainnet,
    Custom(CustomDeployment),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CustomDeployment {
    pub blend: BlendDeploymentSettings,
}
