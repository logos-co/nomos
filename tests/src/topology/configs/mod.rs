pub mod api;
pub mod blend;

pub mod bootstrap;
pub mod consensus;
pub mod da;
pub mod membership;
pub mod network;
pub mod tracing;

pub mod time;

use std::iter::repeat;

use blend::GeneralBlendConfig;
use consensus::GeneralConsensusConfig;
use da::GeneralDaConfig;
use network::GeneralNetworkConfig;
use nomos_utils::net::get_available_udp_port;
use rand::{Rng as _, thread_rng};
use tracing::GeneralTracingConfig;

use crate::topology::configs::{
    api::GeneralApiConfig,
    bootstrap::{GeneralBootstrapConfig, SHORT_PROLONGED_BOOTSTRAP_PERIOD},
    consensus::ConsensusParams,
    da::DaParams,
    membership::{GeneralMembershipConfig, MembershipNode},
    network::NetworkParams,
    time::GeneralTimeConfig,
};

#[derive(Clone)]
pub struct GeneralConfig {
    pub api_config: GeneralApiConfig,
    pub consensus_config: GeneralConsensusConfig,
    pub bootstrapping_config: GeneralBootstrapConfig,
    pub da_config: GeneralDaConfig,
    pub network_config: GeneralNetworkConfig,
    pub membership_config: GeneralMembershipConfig,
    pub blend_config: GeneralBlendConfig,
    pub tracing_config: GeneralTracingConfig,
    pub time_config: GeneralTimeConfig,
}

#[must_use]
pub fn create_general_configs(n_nodes: usize) -> Vec<GeneralConfig> {
    create_general_configs_with_network(n_nodes, &NetworkParams::default())
}

#[must_use]
pub fn create_general_configs_with_network(
    n_nodes: usize,
    network_params: &NetworkParams,
) -> Vec<GeneralConfig> {
    create_general_configs_with_blend_core_subset(n_nodes, n_nodes, network_params)
}

#[must_use]
pub fn create_general_configs_with_blend_core_subset(
    n_nodes: usize,
    n_blend_core_nodes: usize,
    network_params: &NetworkParams,
) -> Vec<GeneralConfig> {
    assert!(
        n_blend_core_nodes <= n_nodes,
        "n_blend_core_nodes({n_blend_core_nodes}) must be less than or equal to n_nodes({n_nodes})",
    );

    let mut ids = vec![[0; 32]; n_nodes];
    let mut da_ports = vec![];
    let mut blend_ports = vec![];

    for id in &mut ids {
        thread_rng().fill(id);
        da_ports.push(get_available_udp_port().unwrap());
        blend_ports.push(get_available_udp_port().unwrap());
    }

    let consensus_params = ConsensusParams::default_for_participants(n_nodes);
    let consensus_configs = consensus::create_consensus_configs(&ids, &consensus_params);
    let bootstrap_config =
        bootstrap::create_bootstrap_configs(&ids, SHORT_PROLONGED_BOOTSTRAP_PERIOD);
    let network_configs = network::create_network_configs(&ids, network_params);
    let da_configs = da::create_da_configs(&ids, &DaParams::default(), &da_ports);
    let api_configs = api::create_api_configs(&ids);
    let blend_configs = blend::create_blend_configs(&ids, &blend_ports);
    let tracing_configs = tracing::create_tracing_configs(&ids);
    let membership_configs = membership::create_membership_configs(
        ids.iter()
            .zip(&da_ports)
            .zip(
                // Take only the first n_blend_core_nodes blend ports.
                blend_ports
                    .iter()
                    .take(n_blend_core_nodes)
                    .map(|&port| Some(port))
                    .chain(repeat(None)),
            )
            .map(|((&id, &da_port), blend_port)| MembershipNode {
                id,
                da_port: Some(da_port),
                blend_port,
            })
            .collect::<Vec<_>>()
            .as_slice(),
    );
    let time_config = time::default_time_config();
    let mut general_configs = vec![];

    for i in 0..n_nodes {
        general_configs.push(GeneralConfig {
            api_config: api_configs[i].clone(),
            consensus_config: consensus_configs[i].clone(),
            bootstrapping_config: bootstrap_config[i].clone(),
            da_config: da_configs[i].clone(),
            network_config: network_configs[i].clone(),
            membership_config: membership_configs[i].clone(),
            blend_config: blend_configs[i].clone(),
            tracing_config: tracing_configs[i].clone(),
            time_config: time_config.clone(),
        });
    }

    general_configs
}
