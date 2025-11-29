use serde::{Deserialize, Serialize};

use crate::config::{
    blend::deployment::Settings as BlendDeploymentSettings,
    cryptarchia::deployment::Settings as CryptarchiaDeploymentSettings,
    network::deployment::Settings as NetworkDeploymentSettings,
};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum WellKnownDeployment {
    #[serde(rename = "mainnet")]
    Mainnet,
}

/// Well-known deployments supported by the Nomos binary.
///
/// Any deployment different than any of the well-known falls under the `Custom`
/// category.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum SerdeSettings {
    WellKnown(WellKnownDeployment),
    Custom(Box<DeploymentSettings>),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(from = "SerdeSettings", into = "SerdeSettings")]
pub struct DeploymentSettings {
    well_known: Option<WellKnownDeployment>,
    pub blend: BlendDeploymentSettings,
    pub network: NetworkDeploymentSettings,
    pub cryptarchia: CryptarchiaDeploymentSettings,
}

impl From<SerdeSettings> for DeploymentSettings {
    fn from(value: SerdeSettings) -> Self {
        match value {
            SerdeSettings::WellKnown(well_known_deployment) => Self {
                blend: well_known_deployment.clone().into(),
                cryptarchia: well_known_deployment.clone().into(),
                network: well_known_deployment.clone().into(),
                well_known: Some(well_known_deployment),
            },
            SerdeSettings::Custom(custom_deployment) => Self {
                blend: custom_deployment.blend,
                cryptarchia: custom_deployment.cryptarchia,
                network: custom_deployment.network,
                well_known: None,
            },
        }
    }
}

impl From<DeploymentSettings> for SerdeSettings {
    fn from(value: DeploymentSettings) -> Self {
        if let Some(well_known_deployment_name) = value.well_known {
            Self::WellKnown(well_known_deployment_name)
        } else {
            Self::Custom(Box::new(value))
        }
    }
}
