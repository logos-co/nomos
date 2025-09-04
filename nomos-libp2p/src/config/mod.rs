pub use identify::Settings as IdentifySettings;
pub use kademlia::Settings as KademliaSettings;
use libp2p::identity::ed25519;
pub use nat::{
    autonat_client::Settings as AutonatClientSettings,
    mapping::Settings as NatMappingSettings,
    Settings as NatSettings,
};
use serde::{Deserialize, Serialize};

use crate::protocol_name::ProtocolName;

mod gossipsub;
mod identify;
mod kademlia;
mod nat;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmConfig {
    /// Listening IPv4 address
    pub host: std::net::Ipv4Addr,
    /// UDP/QUIC listening port. Use 0 for random.
    pub port: u16,
    /// Ed25519 private key in hex format. Default: random.
    #[serde(with = "secret_key_serde", default = "ed25519::SecretKey::generate")]
    pub node_key: ed25519::SecretKey,

    /// Gossipsub config
    #[serde(
        with = "gossipsub::ConfigDef",
        default = "libp2p::gossipsub::Config::default"
    )]
    pub gossipsub_config: libp2p::gossipsub::Config,

    /// Protocol name env for Kademlia and Identify protocol names.
    ///
    /// Allowed values:
    /// - `mainnet`
    /// - `testnet`
    /// - `unittest`
    /// - `integration`
    ///
    /// Default: `unittest`
    #[serde(default)]
    pub protocol_name_env: ProtocolName,

    /// Kademlia config (required; Identify must be enabled too)
    #[serde(default)]
    pub kademlia_config: kademlia::Settings,

    /// Identify config (required)
    #[serde(default)]
    pub identify_config: identify::Settings,

    /// Nat config
    /// When the value is None, NAT is disabled.
    #[serde(default)]
    pub chain_sync_config: cryptarchia_sync::Config,

    /// Nat config
    /// When the value is None, NAT is disabled.
    #[serde(default)]
    pub nat_config: Option<nat::Settings>,
}

impl Default for SwarmConfig {
    fn default() -> Self {
        Self {
            host: std::net::Ipv4Addr::UNSPECIFIED,
            port: 60000,
            node_key: ed25519::SecretKey::generate(),
            gossipsub_config: libp2p::gossipsub::Config::default(),
            protocol_name_env: ProtocolName::default(),
            kademlia_config: kademlia::Settings::default(),
            identify_config: identify::Settings::default(),
            chain_sync_config: cryptarchia_sync::Config::default(),
            nat_config: Some(nat::Settings::default()),
        }
    }
}

pub mod secret_key_serde {
    use libp2p::identity::ed25519;
    use serde::{de::Error as _, Deserialize as _, Deserializer, Serialize as _, Serializer};

    pub fn serialize<S>(key: &ed25519::SecretKey, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let hex_str = hex::encode(key.as_ref());
        hex_str.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<ed25519::SecretKey, D::Error>
    where
        D: Deserializer<'de>,
    {
        let hex_str = String::deserialize(deserializer)?;
        let mut key_bytes = hex::decode(hex_str).map_err(|e| D::Error::custom(format!("{e}")))?;
        ed25519::SecretKey::try_from_bytes(key_bytes.as_mut_slice())
            .map_err(|e| D::Error::custom(format!("{e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_serde() {
        let config = SwarmConfig::default();

        let serialized = serde_json::to_string(&config).unwrap();
        println!("{serialized}");

        let deserialized: SwarmConfig = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized.host, config.host);
        assert_eq!(deserialized.port, config.port);
        assert_eq!(deserialized.node_key.as_ref(), config.node_key.as_ref());
    }
}
