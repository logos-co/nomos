pub use identify::Settings as IdentifySettings;
pub use kademlia::Settings as KademliaSettings;
use libp2p::identity::ed25519;
use serde::{Deserialize, Serialize};

use crate::protocol_name::ProtocolName;

mod gossipsub;
mod identify;
mod kademlia;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmConfig {
    // Listening IPv4 address
    pub host: std::net::Ipv4Addr,
    // TCP listening port. Use 0 for random
    pub port: u16,
    // Secp256k1 private key in Hex format (`0x123...abc`). Default random
    #[serde(with = "secret_key_serde", default = "ed25519::SecretKey::generate")]
    pub node_key: ed25519::SecretKey,
    // Gossipsub config
    #[serde(
        with = "gossipsub::ConfigDef",
        default = "libp2p::gossipsub::Config::default"
    )]
    pub gossipsub_config: libp2p::gossipsub::Config,

    /// Protocol name env for Kademlia and Identify protocol names.
    /// This is used to determine the protocol names for Kademlia and Identify.
    ///
    /// Allowed values are:
    /// - `mainnet`
    /// - `testnet`
    /// - `unittest`
    /// - `integration`
    ///
    /// Default is `unittest`.
    #[serde(default)]
    pub protocol_name_env: ProtocolName,

    /// Kademlia config
    /// Note: Kademlia requires identify or another identity protocol to be
    /// enabled.
    #[serde(default)]
    pub kademlia_config: kademlia::Settings,

    /// Identify config
    #[serde(default)]
    pub identify_config: identify::Settings,

    /// Chain sync config
    #[serde(default)]
    pub chain_sync_config: cryptarchia_sync::Config,
}

impl Default for SwarmConfig {
    fn default() -> Self {
        Self {
            host: std::net::Ipv4Addr::new(0, 0, 0, 0),
            port: 60000,
            node_key: ed25519::SecretKey::generate(),
            gossipsub_config: libp2p::gossipsub::Config::default(),
            protocol_name_env: ProtocolName::default(),
            kademlia_config: kademlia::Settings::default(),
            identify_config: identify::Settings::default(),
            chain_sync_config: cryptarchia_sync::Config::default(),
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

        let deserialized: SwarmConfig = serde_json::from_str(serialized.as_str()).unwrap();
        assert_eq!(deserialized.host, config.host);
        assert_eq!(deserialized.port, config.port);
        assert_eq!(deserialized.node_key.as_ref(), config.node_key.as_ref());
    }
}
