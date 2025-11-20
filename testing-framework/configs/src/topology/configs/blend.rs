use core::time::Duration;
use std::{num::NonZeroU64, str::FromStr as _};

use ed25519_dalek::SigningKey;
use nomos_blend_message::crypto::keys::Ed25519PrivateKey;
use nomos_blend_service::{
    core::backends::libp2p::Libp2pBlendBackendSettings as Libp2pCoreBlendBackendSettings,
    edge::backends::libp2p::Libp2pBlendBackendSettings as Libp2pEdgeBlendBackendSettings,
};
use nomos_libp2p::{Multiaddr, protocol_name::StreamProtocol};
use num_bigint::BigUint;
use zksign::SecretKey;

#[derive(Clone)]
pub struct GeneralBlendConfig {
    pub backend_core: Libp2pCoreBlendBackendSettings,
    pub backend_edge: Libp2pEdgeBlendBackendSettings,
    pub private_key: Ed25519PrivateKey,
    pub secret_zk_key: SecretKey,
    pub signer: SigningKey,
}

/// Builds blend configs for each node.
///
/// # Panics
///
/// Panics if the provided port strings cannot be parsed into valid `Multiaddr`s
/// or if any of the numeric blend parameters are zero, which would make the
/// libp2p configuration invalid.
#[must_use]
pub fn create_blend_configs(ids: &[[u8; 32]], ports: &[u16]) -> Vec<GeneralBlendConfig> {
    ids.iter()
        .zip(ports)
        .map(|(id, port)| {
            let signer = SigningKey::from_bytes(id);

            let private_key = Ed25519PrivateKey::from(*id);
            // We need unique ZK secret keys, so we just derive them deterministically from
            // the generated Ed25519 public keys, which are guaranteed to be unique because
            // they are in turned derived from node ID.
            let secret_zk_key =
                SecretKey::from(BigUint::from_bytes_le(private_key.public_key().as_bytes()));
            GeneralBlendConfig {
                backend_core: Libp2pCoreBlendBackendSettings {
                    listening_address: Multiaddr::from_str(&format!(
                        "/ip4/127.0.0.1/udp/{port}/quic-v1",
                    ))
                    .unwrap(),
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
                    protocol_name: StreamProtocol::new("/blend/integration-tests"),
                    replication_factor: 1.try_into().unwrap(),
                },
                private_key,
                secret_zk_key,
                signer,
            }
        })
        .collect()
}
