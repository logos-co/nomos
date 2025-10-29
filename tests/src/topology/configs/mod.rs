pub mod api;
pub mod blend;

pub mod bootstrap;
pub mod consensus;
pub mod da;
pub mod network;
pub mod tracing;

pub mod time;

use blend::GeneralBlendConfig;
use consensus::{GeneralConsensusConfig, ProviderInfo, create_genesis_tx_with_declarations};
use da::GeneralDaConfig;
use network::GeneralNetworkConfig;
use nomos_core::{
    mantle::{GenesisTx as _, keys::PublicKey},
    sdp::{Locator, ProviderId, ServiceType},
};
use nomos_utils::net::get_available_udp_port;
use num_bigint::BigUint;
use rand::{Rng as _, thread_rng};
use tracing::GeneralTracingConfig;

use crate::topology::configs::{
    api::GeneralApiConfig,
    bootstrap::{GeneralBootstrapConfig, SHORT_PROLONGED_BOOTSTRAP_PERIOD},
    consensus::ConsensusParams,
    da::DaParams,
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

    let mut ids: Vec<_> = (0..n_nodes).map(|i| [i as u8; 32]).collect();
    let mut da_ports = vec![];
    let mut blend_ports = vec![];

    for id in &mut ids {
        thread_rng().fill(id);
        da_ports.push(get_available_udp_port().unwrap());
        blend_ports.push(get_available_udp_port().unwrap());
    }

    let consensus_params = ConsensusParams::default_for_participants(n_nodes);
    let mut consensus_configs = consensus::create_consensus_configs(&ids, &consensus_params);
    let bootstrap_config =
        bootstrap::create_bootstrap_configs(&ids, SHORT_PROLONGED_BOOTSTRAP_PERIOD);
    let network_configs = network::create_network_configs(&ids, network_params);
    let da_configs = da::create_da_configs(&ids, &DaParams::default(), &da_ports);
    let api_configs = api::create_api_configs(&ids);
    let blend_configs = blend::create_blend_configs(&ids, &blend_ports);
    let tracing_configs = tracing::create_tracing_configs(&ids);
    let time_config = time::default_time_config();

    let providers: Vec<_> = blend_configs
        .iter()
        .enumerate()
        .map(|(i, blend_conf)| ProviderInfo {
            service_type: ServiceType::BlendNetwork,
            provider_id: ProviderId(blend_conf.signer.verifying_key()),
            zk_id: PublicKey::new(BigUint::from(0u8).into()),
            locator: Locator(blend_conf.backend_core.listening_address.clone()),
            note: consensus_configs[0].blend_notes[i].clone(),
            signer: blend_conf.signer.clone(),
        })
        .collect();
    let ledger_tx = consensus_configs[0]
        .genesis_tx
        .mantle_tx()
        .ledger_tx
        .clone();
    let genesis_tx = create_genesis_tx_with_declarations(ledger_tx, providers);
    for c in &mut consensus_configs {
        c.genesis_tx = genesis_tx.clone();
    }

    let mut general_configs = vec![];

    for i in 0..n_nodes {
        general_configs.push(GeneralConfig {
            api_config: api_configs[i].clone(),
            consensus_config: consensus_configs[i].clone(),
            bootstrapping_config: bootstrap_config[i].clone(),
            da_config: da_configs[i].clone(),
            network_config: network_configs[i].clone(),
            blend_config: blend_configs[i].clone(),
            tracing_config: tracing_configs[i].clone(),
            time_config: time_config.clone(),
        });
    }

    general_configs
}
