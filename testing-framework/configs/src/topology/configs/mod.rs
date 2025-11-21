pub mod api;
pub mod blend;
pub mod bootstrap;
pub mod consensus;
pub mod da;
pub mod network;
pub mod time;
pub mod tracing;
pub mod wallet;

use blend::GeneralBlendConfig;
use consensus::{GeneralConsensusConfig, ProviderInfo, create_genesis_tx_with_declarations};
use da::GeneralDaConfig;
use key_management_system::{
    backend::preload::PreloadKMSBackendSettings,
    keys::{Ed25519Key, Key, ZkKey},
};
use network::GeneralNetworkConfig;
use nomos_core::{
    mantle::GenesisTx as _,
    sdp::{Locator, ServiceType},
};
use nomos_utils::net::get_available_udp_port;
use rand::{Rng as _, thread_rng};
use tracing::GeneralTracingConfig;
use wallet::WalletConfig;

use crate::{
    common::kms::key_id_for_preload_backend,
    topology::configs::{
        api::GeneralApiConfig,
        bootstrap::{GeneralBootstrapConfig, SHORT_PROLONGED_BOOTSTRAP_PERIOD},
        consensus::ConsensusParams,
        da::DaParams,
        network::NetworkParams,
        time::GeneralTimeConfig,
    },
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
    pub kms_config: PreloadKMSBackendSettings,
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
    // TODO: Instead of this, define a config struct for each node.
    // That would be also useful for non-even token distributions: https://github.com/logos-co/nomos/issues/1888
    n_blend_core_nodes: usize,
    network_params: &NetworkParams,
) -> Vec<GeneralConfig> {
    assert!(
        n_blend_core_nodes <= n_nodes,
        "n_blend_core_nodes({n_blend_core_nodes}) must be less than or equal to n_nodes({n_nodes})",
    );

    // Blend relies on each node declaring a different ZK public key, so we need
    // different IDs to generate different keys.
    let mut ids: Vec<_> = (0..n_nodes).map(|i| [i as u8; 32]).collect();
    let mut da_ports = vec![];
    let mut blend_ports = vec![];

    for id in &mut ids {
        thread_rng().fill(id);
        da_ports.push(get_available_udp_port().unwrap());
        blend_ports.push(get_available_udp_port().unwrap());
    }

    let consensus_params = ConsensusParams::default_for_participants(n_nodes);
    let mut consensus_configs =
        consensus::create_consensus_configs(&ids, &consensus_params, &WalletConfig::default());
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
        .take(n_blend_core_nodes)
        .map(|(i, blend_conf)| ProviderInfo {
            service_type: ServiceType::BlendNetwork,
            provider_sk: blend_conf.signer.clone(),
            zk_sk: blend_conf.secret_zk_key.clone(),
            locator: Locator(blend_conf.backend_core.listening_address.clone()),
            note: consensus_configs[0].blend_notes[i].clone(),
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

    // Set Blend and DA keys in KMS of each node config.
    let kms_configs: Vec<_> = blend_configs
        .iter()
        .map(|blend_conf| {
            let ed_key = Ed25519Key::new(blend_conf.signer.clone());
            let zk_key = ZkKey::new(blend_conf.secret_zk_key.clone());
            PreloadKMSBackendSettings {
                keys: [
                    (
                        key_id_for_preload_backend(&Key::from(ed_key.clone())),
                        Key::from(ed_key),
                    ),
                    (
                        key_id_for_preload_backend(&Key::from(zk_key.clone())),
                        Key::from(zk_key),
                    ),
                ]
                .into(),
            }
        })
        .collect();

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
            kms_config: kms_configs[i].clone(),
        });
    }

    general_configs
}
