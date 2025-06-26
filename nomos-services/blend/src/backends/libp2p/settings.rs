use std::num::NonZeroU64;

use libp2p::Multiaddr;
use nomos_libp2p::ed25519;
use nomos_utils::math::NonNegativeF64;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Libp2pBlendBackendSettings {
    pub listening_address: Multiaddr,
    // A key for deriving PeerId and establishing secure connections (TLS 1.3 by QUIC)
    #[serde(
        with = "nomos_libp2p::secret_key_serde",
        default = "ed25519::SecretKey::generate"
    )]
    pub node_key: ed25519::SecretKey,
    pub peering_degree: usize,
    pub max_peering_degree: u32,
    pub minimum_messages_coefficient: NonZeroU64,
    pub normalization_constant: NonNegativeF64,
}
