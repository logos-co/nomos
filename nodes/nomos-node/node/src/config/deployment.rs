use nomos_blend_service::settings::DeploymentSettings as BlendDeploymentSettings;
use serde::Deserialize;

#[derive(Deserialize, Debug, Clone, Serialize)]
pub enum DeploymentConfig<BlendDeploymentConfig> {
    Mainnet,
    Testnet,
    Custom(CustomDeploymentConfig),
}

#[derive(Deserialize, Debug, Clone, Serialize)]
pub struct CustomDeploymentConfig {
    pub blend: DeploymentSettings,
}
