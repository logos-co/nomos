use core::time::Duration;
use std::{num::NonZeroU64, ops::RangeInclusive};

use libp2p::{Multiaddr, PeerId, identity::Keypair};
use nomos_libp2p::{ed25519, protocol_name::StreamProtocol};
use nomos_utils::math::NonNegativeF64;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde_with::serde_as]
pub struct Libp2pBlendBackendSettings {
    pub listening_address: Multiaddr,
    // A key for deriving PeerId and establishing secure connections (TLS 1.3 by QUIC)
    #[serde(
        with = "nomos_libp2p::secret_key_serde",
        default = "ed25519::SecretKey::generate"
    )]
    pub node_key: ed25519::SecretKey,
    pub core_peering_degree: RangeInclusive<u64>,
    pub minimum_messages_coefficient: NonZeroU64,
    pub normalization_constant: NonNegativeF64,
    #[serde_as(
        as = "nomos_utils::bounded_duration::MinimalBoundedDuration<1, nomos_utils::bounded_duration::SECOND>"
    )]
    pub edge_node_connection_timeout: Duration,
    pub max_edge_node_incoming_connections: u64,
    pub max_dial_attempts_per_peer: NonZeroU64,
    pub protocol_name: StreamProtocol,
}

impl Libp2pBlendBackendSettings {
    #[must_use]
    pub fn keypair(&self) -> Keypair {
        Keypair::from(ed25519::Keypair::from(self.node_key.clone()))
    }

    #[must_use]
    pub fn peer_id(&self) -> PeerId {
        self.keypair().public().to_peer_id()
    }
}
