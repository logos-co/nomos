pub mod configs;

use std::time::Duration;

use configs::{
    da::{create_da_configs, DaParams},
    network::{create_network_configs, NetworkParams},
    tracing::create_tracing_configs,
    GeneralConfig,
};
use nomos_da_network_core::swarm::DAConnectionPolicySettings;
use nomos_utils::net::get_available_udp_port;
use rand::{thread_rng, Rng as _};

use crate::{
    nodes::{
        executor::{create_executor_config, Executor},
        validator::{create_validator_config, Validator},
    },
    topology::configs::{
        api::create_api_configs,
        blend::create_blend_configs,
        bootstrap::create_bootstrap_configs,
        consensus::{create_consensus_configs, ConsensusParams},
        membership::{create_empty_da_membership_configs, create_membership_configs},
        time::default_time_config,
    },
};

pub struct TopologyConfig {
    pub n_validators: usize,
    pub n_executors: usize,
    pub consensus_params: ConsensusParams,
    pub da_params: DaParams,
    pub network_params: NetworkParams,
}

impl TopologyConfig {
    #[must_use]
    pub fn two_validators() -> Self {
        Self {
            n_validators: 2,
            n_executors: 0,
            consensus_params: ConsensusParams::default_for_participants(2),
            da_params: DaParams::default(),
            network_params: NetworkParams::default(),
        }
    }

    #[must_use]
    pub fn validator_and_executor() -> Self {
        Self {
            n_validators: 1,
            n_executors: 1,
            consensus_params: ConsensusParams::default_for_participants(2),
            da_params: DaParams {
                dispersal_factor: 2,
                subnetwork_size: 2,
                num_subnets: 2,
                policy_settings: DAConnectionPolicySettings {
                    min_dispersal_peers: 1,
                    min_replication_peers: 1,
                    max_dispersal_failures: 0,
                    max_sampling_failures: 0,
                    max_replication_failures: 0,
                    malicious_threshold: 0,
                },
                balancer_interval: Duration::from_secs(1),
                ..Default::default()
            },
            network_params: NetworkParams::default(),
        }
    }

    #[must_use]
    pub fn validators_and_executor(
        num_validators: usize,
        num_subnets: usize,
        dispersal_factor: usize,
    ) -> Self {
        Self {
            n_validators: num_validators,
            n_executors: 1,
            consensus_params: ConsensusParams::default_for_participants(num_validators + 1),
            da_params: DaParams {
                dispersal_factor,
                subnetwork_size: num_subnets,
                num_subnets: num_subnets as u16,
                policy_settings: DAConnectionPolicySettings {
                    min_dispersal_peers: num_subnets,
                    min_replication_peers: dispersal_factor - 1,
                    max_dispersal_failures: 0,
                    max_sampling_failures: 0,
                    max_replication_failures: 0,
                    malicious_threshold: 0,
                },
                balancer_interval: Duration::from_secs(5),
                ..Default::default()
            },
            network_params: NetworkParams::default(),
        }
    }
}

pub struct Topology {
    validators: Vec<Validator>,
    executors: Vec<Executor>,
}

impl Topology {
    pub async fn spawn(config: TopologyConfig) -> Self {
        let n_participants = config.n_validators + config.n_executors;

        // we use the same random bytes for:
        // * da id
        // * coin sk
        // * coin nonce
        // * libp2p node key
        let mut ids = vec![[0; 32]; n_participants];
        let mut da_ports = vec![];
        let mut blend_ports = vec![];
        for id in &mut ids {
            thread_rng().fill(id);
            da_ports.push(get_available_udp_port().unwrap());
            blend_ports.push(get_available_udp_port().unwrap());
        }

        let consensus_configs = create_consensus_configs(&ids, &config.consensus_params);
        let bootstrapping_config = create_bootstrap_configs(&ids, Duration::from_secs(30));
        let da_configs = create_da_configs(&ids, &config.da_params, &da_ports);
        let membership_configs = create_membership_configs(ids.as_slice(), &da_ports, &blend_ports);
        let network_configs = create_network_configs(&ids, &config.network_params);
        let blend_configs = create_blend_configs(&ids, &blend_ports);
        let api_configs = create_api_configs(&ids);
        let tracing_configs = create_tracing_configs(&ids);
        let time_config = default_time_config();

        let mut node_configs = vec![];

        for i in 0..n_participants {
            node_configs.push(GeneralConfig {
                consensus_config: consensus_configs[i].clone(),
                bootstrapping_config: bootstrapping_config[i].clone(),
                da_config: da_configs[i].clone(),
                network_config: network_configs[i].clone(),
                blend_config: blend_configs[i].clone(),
                api_config: api_configs[i].clone(),
                tracing_config: tracing_configs[i].clone(),
                time_config: time_config.clone(),
                membership_config: membership_configs[i].clone(),
            });
        }

        let (validators, executors) =
            Self::spawn_validators_executors(node_configs, config.n_validators, config.n_executors)
                .await;

        Self {
            validators,
            executors,
        }
    }

    pub async fn spawn_with_empty_membership(
        config: TopologyConfig,
        ids: &[[u8; 32]],
        da_ports: &[u16],
        blend_ports: &[u16],
    ) -> Self {
        let n_participants = config.n_validators + config.n_executors;

        let consensus_configs = create_consensus_configs(ids, &config.consensus_params);
        let bootstrapping_config = create_bootstrap_configs(ids, Duration::from_secs(60));
        let da_configs = create_da_configs(ids, &config.da_params, da_ports);
        let network_configs = create_network_configs(ids, &config.network_params);
        let blend_configs = create_blend_configs(ids, blend_ports);
        let api_configs = create_api_configs(ids);
        let membership_configs = create_empty_da_membership_configs(ids, blend_ports);
        let tracing_configs = create_tracing_configs(ids);
        let time_config = default_time_config();

        let mut node_configs = vec![];

        for i in 0..n_participants {
            node_configs.push(GeneralConfig {
                consensus_config: consensus_configs[i].clone(),
                bootstrapping_config: bootstrapping_config[i].clone(),
                da_config: da_configs[i].clone(),
                network_config: network_configs[i].clone(),
                blend_config: blend_configs[i].clone(),
                api_config: api_configs[i].clone(),
                tracing_config: tracing_configs[i].clone(),
                time_config: time_config.clone(),
                membership_config: membership_configs[i].clone(),
            });
        }
        let (validators, executors) =
            Self::spawn_validators_executors(node_configs, config.n_validators, config.n_executors)
                .await;

        Self {
            validators,
            executors,
        }
    }

    async fn spawn_validators_executors(
        config: Vec<GeneralConfig>,
        n_validators: usize,
        n_executors: usize,
    ) -> (Vec<Validator>, Vec<Executor>) {
        let mut validators = Vec::new();
        for i in 0..n_validators {
            let config = create_validator_config(config[i].clone());
            validators.push(Validator::spawn(config).await.unwrap());
        }

        let mut executors = Vec::new();
        for i in n_validators..(n_validators + n_executors) {
            let config = create_executor_config(config[i].clone());
            executors.push(Executor::spawn(config).await);
        }

        (validators, executors)
    }

    #[must_use]
    pub fn validators(&self) -> &[Validator] {
        &self.validators
    }

    #[must_use]
    pub fn executors(&self) -> &[Executor] {
        &self.executors
    }
}
