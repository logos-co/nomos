use std::str::FromStr as _;

use nomos_blend::membership::Node;
use nomos_blend_message::{sphinx::SphinxMessage, BlendMessage};
use nomos_blend_service::backends::libp2p::Libp2pBlendBackendSettings;
use nomos_libp2p::{
    ed25519::{self, Keypair as Ed25519Keypair},
    identity::Keypair,
    Multiaddr, PeerId,
};

use crate::get_available_port;

#[derive(Clone)]
pub struct GeneralBlendConfig {
    pub backend: Libp2pBlendBackendSettings,
    pub private_key: x25519_dalek::StaticSecret,
    pub membership: Vec<Node<PeerId, <SphinxMessage as BlendMessage>::PublicKey>>,
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
                },
                private_key: x25519_dalek::StaticSecret::random(),
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

fn blend_nodes(
    configs: &[GeneralBlendConfig],
) -> Vec<Node<PeerId, <SphinxMessage as BlendMessage>::PublicKey>> {
    configs
        .iter()
        .map(|config| Node {
            id: PeerId::from_public_key(
                &Keypair::from(Ed25519Keypair::from(config.backend.node_key.clone())).public(),
            ),
            address: config.backend.listening_address.clone(),
            public_key: x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from(
                config.private_key.to_bytes(),
            ))
            .to_bytes(),
        })
        .collect()
}
