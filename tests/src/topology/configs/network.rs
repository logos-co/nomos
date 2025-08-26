use std::time::Duration;

use nomos_libp2p::{ed25519, Multiaddr, SwarmConfig};

use crate::{get_available_port, node_address_from_port};

#[derive(Default)]
pub enum Libp2pNetworkLayout {
    #[default]
    Star,
    Chain,
}

#[derive(Default)]
pub struct NetworkParams {
    pub libp2p_network_layout: Libp2pNetworkLayout,
}

#[derive(Clone)]
pub struct GeneralNetworkConfig {
    pub swarm_config: SwarmConfig,
    pub initial_peers: Vec<Multiaddr>,
}

#[must_use]
pub fn create_network_configs(
    ids: &[[u8; 32]],
    network_params: &NetworkParams,
) -> Vec<GeneralNetworkConfig> {
    let swarm_configs: Vec<SwarmConfig> = ids
        .iter()
        .map(|id| {
            let mut node_key_bytes = *id;
            let node_key = ed25519::SecretKey::try_from_bytes(&mut node_key_bytes)
                .expect("Failed to generate secret key from bytes");

            SwarmConfig {
                node_key,
                port: get_available_port(),
                chain_sync_config: cryptarchia_sync::Config {
                    peer_response_timeout: Duration::from_secs(60),
                },
                ..Default::default()
            }
        })
        .collect();

    let all_initial_peers = initial_peers_by_network_layout(swarm_configs.clone(), network_params);

    swarm_configs
        .iter()
        .zip(all_initial_peers)
        .map(|(swarm_config, initial_peers)| GeneralNetworkConfig {
            swarm_config: swarm_config.to_owned(),
            initial_peers,
        })
        .collect()
}

fn initial_peers_by_network_layout(
    mut swarm_configs: Vec<SwarmConfig>,
    network_params: &NetworkParams,
) -> Vec<Vec<Multiaddr>> {
    let mut all_initial_peers = vec![];
    let first_swarm = swarm_configs.remove(0);
    let first_addr = node_address_from_port(first_swarm.port);

    match network_params.libp2p_network_layout {
        Libp2pNetworkLayout::Star => {
            let other_initial_peers = vec![first_addr];
            all_initial_peers.push(vec![]); // First node has no initial peers.

            for _ in swarm_configs {
                all_initial_peers.push(other_initial_peers.clone());
            }
        }
        Libp2pNetworkLayout::Chain => {
            let mut prev_addr = first_addr;
            all_initial_peers.push(vec![]); // First node has no initial peers.

            for swarm in swarm_configs {
                all_initial_peers.push(vec![prev_addr]);
                prev_addr = node_address_from_port(swarm.port);
            }
        }
    }

    all_initial_peers
}
