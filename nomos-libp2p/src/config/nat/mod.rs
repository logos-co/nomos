use serde::{Deserialize, Serialize};

pub mod autonat_client;
pub mod gateway;
pub mod mapping;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct Settings {
    pub autonat: autonat_client::Settings,
    pub mapping: mapping::Settings,
    pub gateway_monitor: gateway::Settings,
}
