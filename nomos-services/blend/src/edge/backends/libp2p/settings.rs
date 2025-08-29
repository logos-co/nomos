use core::num::NonZeroU64;

use libp2p::identity::Keypair;
use nomos_libp2p::{ed25519, protocol_name::StreamProtocol};
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
    pub max_dial_attempts_per_peer_per_message: NonZeroU64,
    pub protocol_name: StreamProtocol,
    // $\Phi_{EC}$: the minimum number of connections that the edge node establishes with
    // core nodes to send a single message that needs to be blended.
    pub replication_factor: NonZeroU64,
}

impl Libp2pBlendBackendSettings {
    #[must_use]
    pub fn keypair(&self) -> Keypair {
        Keypair::from(ed25519::Keypair::from(self.node_key.clone()))
    }
}
