use core::time::Duration;
use std::{num::NonZeroU64, str::FromStr as _};

use ed25519_dalek::SigningKey;
use nomos_blend_message::crypto::keys::Ed25519PrivateKey;
use nomos_blend_service::{
    core::backends::libp2p::Libp2pBlendBackendSettings as Libp2pCoreBlendBackendSettings,
    edge::backends::libp2p::Libp2pBlendBackendSettings as Libp2pEdgeBlendBackendSettings,
};
use nomos_core::{mantle::keys::SecretKey, sdp::ProviderId};
use nomos_libp2p::{
    Multiaddr,
    ed25519::{self},
    protocol_name::StreamProtocol,
};
use num_bigint::BigUint;

#[derive(Clone)]
pub struct GeneralBlendConfig {
    pub backend_core: Libp2pCoreBlendBackendSettings,
    pub backend_edge: Libp2pEdgeBlendBackendSettings,
    pub private_key: Ed25519PrivateKey,
    pub secret_zk_key: SecretKey,
    pub provider_id: ProviderId,
}

#[must_use]
pub fn create_blend_configs(ids: &[[u8; 32]], ports: &[u16]) -> Vec<GeneralBlendConfig> {
    ids.iter()
        .zip(ports)
        .map(|(id, port)| {
            let mut node_key_bytes = *id;
            let node_key = ed25519::SecretKey::try_from_bytes(&mut node_key_bytes)
                .expect("Failed to generate secret key from bytes");
            let dalek_signing_key = SigningKey::from_bytes(id);
            let provider_id = ProviderId(dalek_signing_key.verifying_key());

            let private_key = Ed25519PrivateKey::from(*id);
            let secret_zk_key =
                SecretKey::from(BigUint::from_bytes_le(private_key.public_key().as_bytes()));
            GeneralBlendConfig {
                backend_core: Libp2pCoreBlendBackendSettings {
                    listening_address: Multiaddr::from_str(&format!(
                        "/ip4/127.0.0.1/udp/{port}/quic-v1",
                    ))
                    .unwrap(),
                    node_key: node_key.clone(),
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
                backend_edge: Libp2pEdgeBlendBackendSettings {
                    max_dial_attempts_per_peer_per_message: 1.try_into().unwrap(),
                    node_key,
                    protocol_name: StreamProtocol::new("/blend/integration-tests"),
                    replication_factor: 1.try_into().unwrap(),
                },
                private_key,
                secret_zk_key,
                provider_id,
            }
        })
        .collect()
}
