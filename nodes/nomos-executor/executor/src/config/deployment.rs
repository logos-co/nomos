use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CustomDeployment {
    pub blend: nomos_node::config::blend::deployment::Settings,
    pub network: nomos_node::config::network::deployment::Settings,
    pub da: crate::config::da::deployment::Settings,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Settings {
    #[serde(rename = "mainnet")]
    Mainnet,
    Custom(Box<CustomDeployment>),
}

impl From<Settings> for nomos_node::config::deployment::Settings {
    fn from(value: Settings) -> Self {
        match value {
            Settings::Mainnet => Self::Mainnet,
            Settings::Custom(custom) => {
                Self::Custom(Box::new(nomos_node::config::deployment::CustomDeployment {
                    blend: custom.blend,
                    network: custom.network,
                    // Convert executor DA deployment to validator DA deployment
                    // (strip dispersal since validator doesn't need it)
                    da: nomos_node::config::da::deployment::Settings {
                        common: custom.da.common,
                        network: custom.da.network,
                        verifier: custom.da.verifier,
                        sampling: custom.da.sampling,
                    },
                }))
            }
        }
    }
}
