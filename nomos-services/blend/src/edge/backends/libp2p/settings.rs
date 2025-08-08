use libp2p::identity::Keypair;
use nomos_libp2p::ed25519;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde_with::serde_as]
pub struct Libp2pBlendBackendSettings {
    // A key for deriving PeerId and establishing secure connections (TLS 1.3 by QUIC)
    #[serde(
        with = "nomos_libp2p::secret_key_serde",
        default = "ed25519::SecretKey::generate"
    )]
    pub node_key: ed25519::SecretKey,
}

impl Libp2pBlendBackendSettings {
    #[must_use]
    pub fn keypair(&self) -> Keypair {
        Keypair::from(ed25519::Keypair::from(self.node_key.clone()))
    }
}
