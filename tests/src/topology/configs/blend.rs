use core::time::Duration;
use std::{num::NonZeroU64, str::FromStr as _};

use nomos_blend_message::crypto::keys::Ed25519PrivateKey;
use nomos_blend_service::core::backends::libp2p::Libp2pBlendBackendSettings;
use nomos_libp2p::{
    ed25519::{self},
    protocol_name::StreamProtocol,
    Multiaddr,
};

#[derive(Clone)]
pub struct GeneralBlendConfig {
    pub backend: Libp2pBlendBackendSettings,
    pub private_key: Ed25519PrivateKey,
}

#[must_use]
pub fn create_blend_configs(ids: &[[u8; 32]], ports: &[u16]) -> Vec<GeneralBlendConfig> {
    ids.iter()
        .zip(ports)
        .map(|(id, port)| {
            let mut node_key_bytes = *id;
            let node_key = ed25519::SecretKey::try_from_bytes(&mut node_key_bytes)
                .expect("Failed to generate secret key from bytes");

            GeneralBlendConfig {
                backend: Libp2pBlendBackendSettings {
                    listening_address: Multiaddr::from_str(&format!(
                        "/ip4/127.0.0.1/udp/{port}/quic-v1",
                    ))
                    .unwrap(),
                    node_key,
                    core_peering_degree: 1..=3,
                    minimum_messages_coefficient: NonZeroU64::try_from(1)
                        .expect("Minimum messages coefficient cannot be zero."),
                    normalization_constant: 1.03f64
                        .try_into()
                        .expect("Normalization constant cannot be negative."),
                    edge_node_connection_timeout: Duration::from_secs(1),
                    max_edge_node_incoming_connections: 300,
                    max_dial_attempts_per_peer: NonZeroU64::try_from(3)
                        .expect("Max dial attempts per peer cannot be zero."),
                    protocol_name: StreamProtocol::new("/blend/integration-tests"),
                },
                private_key: Ed25519PrivateKey::from(*id),
            }
        })
        .collect()
}
