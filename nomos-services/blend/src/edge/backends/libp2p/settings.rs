use nomos_libp2p::ed25519;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde_with::serde_as]
pub struct Libp2pBlendEdgeBackendSettings {
    // A key for deriving PeerId and establishing secure connections (TLS 1.3 by QUIC)
    #[serde(
        with = "nomos_libp2p::secret_key_serde",
        default = "ed25519::SecretKey::generate"
    )]
    pub node_key: ed25519::SecretKey,
}
