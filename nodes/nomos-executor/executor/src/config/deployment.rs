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
                    da: custom.da.validator,
                }))
            }
        }
    }
}
