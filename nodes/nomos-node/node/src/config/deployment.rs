use serde::{Deserialize, Serialize};

use crate::config::{
    blend::deployment::Settings as BlendDeploymentSettings,
    cryptarchia::deployment::Settings as CryptarchiaDeploymentSettings,
    network::deployment::Settings as NetworkDeploymentSettings,
    time::deployment::Settings as TimeDeploymentSettings,
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
    Custom {
        blend: Box<BlendDeploymentSettings>,
        network: NetworkDeploymentSettings,
        cryptarchia: CryptarchiaDeploymentSettings,
        time: TimeDeploymentSettings,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(from = "SerdeSettings", into = "SerdeSettings")]
pub struct DeploymentSettings {
    well_known: Option<WellKnownDeployment>,
    pub blend: BlendDeploymentSettings,
    pub network: NetworkDeploymentSettings,
    pub cryptarchia: CryptarchiaDeploymentSettings,
    pub time: TimeDeploymentSettings,
}

impl DeploymentSettings {
    #[must_use]
    pub const fn new_custom(
        blend: BlendDeploymentSettings,
        network: NetworkDeploymentSettings,
        cryptarchia: CryptarchiaDeploymentSettings,
        time: TimeDeploymentSettings,
    ) -> Self {
        Self {
            well_known: None,
            blend,
            network,
            cryptarchia,
            time,
        }
    }
}

impl From<WellKnownDeployment> for DeploymentSettings {
    fn from(value: WellKnownDeployment) -> Self {
        Self {
            blend: value.clone().into(),
            cryptarchia: value.clone().into(),
            network: value.clone().into(),
            time: value.clone().into(),
            well_known: Some(value),
        }
    }
}

impl From<SerdeSettings> for DeploymentSettings {
    fn from(value: SerdeSettings) -> Self {
        match value {
            SerdeSettings::WellKnown(well_known_deployment) => well_known_deployment.into(),
            SerdeSettings::Custom {
                blend,
                cryptarchia,
                network,
                time,
            } => Self::new_custom(*blend, network, cryptarchia, time),
        }
    }
}

impl From<DeploymentSettings> for SerdeSettings {
    fn from(
        DeploymentSettings {
            blend,
            cryptarchia,
            network,
            time,
            well_known,
        }: DeploymentSettings,
    ) -> Self {
        well_known.map_or_else(
            || Self::Custom {
                blend: Box::new(blend),
                cryptarchia,
                network,
                time,
            },
            Self::WellKnown,
        )
    }
}
