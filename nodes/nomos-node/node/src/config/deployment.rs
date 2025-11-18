use serde::{Deserialize, Serialize};

use crate::config::blend::deployment::Settings as BlendDeploymentSettings;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Settings {
    Mainnet,
    Custom(CustomDeployment),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CustomDeployment {
    pub blend: BlendDeploymentSettings,
}
