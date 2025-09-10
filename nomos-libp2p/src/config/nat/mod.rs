use serde::{Deserialize, Serialize};

pub mod autonat_client;
pub mod gateway;
pub mod mapping;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Settings {
    pub autonat: autonat_client::Settings,
    pub mapping: mapping::Settings,
    pub gateway_monitor: gateway::Settings,
    /// Static external address for nodes with fixed public IPs
    /// When set, NAT traversal is disabled and this address is used directly
    #[serde(default)]
    pub static_external_address: Option<libp2p::Multiaddr>,
}
