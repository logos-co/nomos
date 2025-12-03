use nomos_libp2p::{Multiaddr, ed25519};
use serde::{Deserialize, Serialize};

/// User-configurable DA network settings (node-specific).
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Config {
    /// Node's secret key
    #[serde(
        with = "nomos_libp2p::secret_key_serde",
        default = "ed25519::SecretKey::generate"
    )]
    pub node_key: ed25519::SecretKey,

    /// Listening address for DA network
    pub listening_address: Multiaddr,

    /// API adapter port
    pub api_port: u16,

    /// Whether API is secure
    #[serde(default)]
    pub is_secure: bool,
}
