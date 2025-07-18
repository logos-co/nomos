use core::time::Duration;
use std::{num::NonZeroU64, str::FromStr as _};

use nomos_blend_message::crypto::Ed25519PrivateKey;
use nomos_blend_scheduling::membership::Node;
use nomos_blend_service::core::backends::libp2p::Libp2pBlendBackendSettings;
use nomos_libp2p::{
    ed25519::{self, Keypair as Ed25519Keypair},
    identity::Keypair,
    Multiaddr, PeerId,
};

use crate::get_available_port;

#[derive(Clone)]
pub struct GeneralBlendConfig {
    pub backend: Libp2pBlendBackendSettings,
    pub private_key: Ed25519PrivateKey,
    pub membership: Vec<Node<PeerId>>,
}

#[must_use]
pub fn create_blend_configs(ids: &[[u8; 32]]) -> Vec<GeneralBlendConfig> {
    let mut configs: Vec<GeneralBlendConfig> = ids
        .iter()
        .map(|id| {
            let mut node_key_bytes = *id;
            let node_key = ed25519::SecretKey::try_from_bytes(&mut node_key_bytes)
                .expect("Failed to generate secret key from bytes");

            GeneralBlendConfig {
                backend: Libp2pBlendBackendSettings {
                    listening_address: Multiaddr::from_str(&format!(
                        "/ip4/127.0.0.1/udp/{}/quic-v1",
                        get_available_port(),
                    ))
                    .unwrap(),
                    node_key,
                    peering_degree: 1,
                    max_peering_degree: 3,
                    minimum_messages_coefficient: NonZeroU64::try_from(1)
                        .expect("Minimum messages coefficient cannot be zero."),
                    normalization_constant: 1.03f64
                        .try_into()
                        .expect("Normalization constant cannot be negative."),
                    edge_node_connection_timeout: Duration::from_secs(1),
                },
                private_key: Ed25519PrivateKey::generate(),
                membership: Vec::new(),
            }
        })
        .collect();

    let nodes = blend_nodes(&configs);
    for config in &mut configs {
        config.membership.clone_from(&nodes);
    }

    configs
}

fn blend_nodes(configs: &[GeneralBlendConfig]) -> Vec<Node<PeerId>> {
    configs
        .iter()
        .map(|config| Node {
            id: PeerId::from_public_key(
                &Keypair::from(Ed25519Keypair::from(config.backend.node_key.clone())).public(),
            ),
            address: config.backend.listening_address.clone(),
            public_key: config.private_key.public_key(),
        })
        .collect()
}
